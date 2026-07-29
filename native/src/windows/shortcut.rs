//! Resolve Windows `.lnk` shortcuts via the shell `IShellLinkW` COM interface.
//!
//! This replaces the `lnk` crate, which panicked on many real shortcuts and
//! corrupted non-ASCII target paths (turning `C:\Users\Макс\...` into
//! `C:\Users\????\...`), causing launches to fail with "path not found". The
//! shell API resolves the real target, arguments, working directory and icon in
//! correct UTF-16, which lets us CreateProcessW the app directly onto the hidden
//! desktop.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use windows::core::{Interface, PCWSTR};

use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLGP_RAWPATH};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// The interesting fields extracted from a `.lnk` shortcut.
pub struct ResolvedShortcut {
    pub target: String,
    pub arguments: String,
    pub working_dir: String,
    pub icon: Option<String>,
}

thread_local! {
    /// COM must be initialised once per thread before using shell interfaces.
    /// `desktop_apps` is built on the thread that constructs `WindowMod`, so we
    /// initialise lazily on first use and never uninitialise (process lifetime).
    static COM_INIT: bool = init_com();
}

fn init_com() -> bool {
    // COINIT_APARTMENTTHREADED is required for shell objects. A prior
    // initialisation with the same model returns S_FALSE, which is fine.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    hr.is_ok()
}

/// Read a wide buffer that the shell filled up to a NUL terminator into a
/// Rust `String`. Stops at the first NUL.
fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
        .to_string_lossy()
        .into_owned()
}

/// Resolve a `.lnk` file's target, arguments, working directory and icon.
///
/// Returns `None` only if COM/shell initialisation or loading the link fails.
/// Individual fields default to empty strings when the shortcut does not set
/// them. This function never panics.
pub fn resolve_lnk(path: &Path) -> Option<ResolvedShortcut> {
    // Ensure COM is initialised on this thread.
    let ok = COM_INIT.with(|&v| v);
    if !ok {
        eprintln!("[windowmod][lnk] CoInitializeEx failed");
        return None;
    }

    // Wide, NUL-terminated path to the .lnk file.
    let mut wpath: Vec<u16> = path.as_os_str().encode_wide().collect();
    wpath.push(0);

    unsafe {
        // Create the ShellLink COM object and its IPersistFile to load the .lnk.
        let link: IShellLinkW =
            match CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[windowmod][lnk] CoCreateInstance failed: {e}");
                    return None;
                }
            };

        let persist: IPersistFile = match link.cast() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[windowmod][lnk] cast to IPersistFile failed: {e}");
                return None;
            }
        };

        if let Err(e) = persist.Load(PCWSTR(wpath.as_ptr()), STGM_READ) {
            eprintln!("[windowmod][lnk] IPersistFile::Load failed for {:?}: {e}", path);
            return None;
        }

        // Resolve the link. SLR_NO_UI | SLR_NOUPDATE-like behaviour: we just
        // want the stored target. Resolve may hit the network for dead links;
        // pass flags to avoid UI and excessive search. A failure here is not
        // fatal — GetPath below often still returns the stored path.
        // SLR_NO_UI = 0x1, SLR_NOUPDATE = 0x8, SLR_NOSEARCH = 0x10.
        let _ = link.Resolve(None, 0x1 | 0x8 | 0x10);

        // Target path (raw, so environment variables and the literal stored
        // path are preserved without mangling).
        let mut target_buf = [0u16; MAX_PATH as usize];
        let _ = link.GetPath(
            &mut target_buf,
            std::ptr::null_mut(),
            SLGP_RAWPATH.0 as u32,
        );
        let target = wide_to_string(&target_buf);

        // Command-line arguments (e.g. Discord's `--processStart Discord.exe`).
        let mut args_buf = [0u16; 1024];
        let _ = link.GetArguments(&mut args_buf);
        let arguments = wide_to_string(&args_buf);

        // Working directory.
        let mut dir_buf = [0u16; MAX_PATH as usize];
        let _ = link.GetWorkingDirectory(&mut dir_buf);
        let working_dir = wide_to_string(&dir_buf);

        // Icon location (file path; index ignored — we only render the file).
        let mut icon_buf = [0u16; MAX_PATH as usize];
        let mut icon_index = 0i32;
        let _ = link.GetIconLocation(&mut icon_buf, &mut icon_index);
        let icon_str = wide_to_string(&icon_buf);
        let icon = if icon_str.is_empty() {
            None
        } else {
            Some(icon_str)
        };

        // Keep SW_SHOWNORMAL referenced so unused-import lints stay quiet if a
        // future refactor drops the show-command read.
        let _ = SW_SHOWNORMAL;

        Some(ResolvedShortcut {
            target,
            arguments,
            working_dir,
            icon,
        })
    }
}
