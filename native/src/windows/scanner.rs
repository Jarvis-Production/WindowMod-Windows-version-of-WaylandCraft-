//! Background window-scanning thread.
//!
//! WHY THIS EXISTS (the "apps still lag / browser freezes" fix):
//! The window-discovery passes (launcher-children, popups, off-screen re-hide)
//! call blocking Win32 APIs — `EnumDesktopWindows`, `EnumWindows`,
//! `GetClientRect`, `GetWindow`, `GetClassNameW` — on windows that may be BUSY
//! (Opera playing a video, a loading game). Those calls can block for tens to
//! hundreds of milliseconds. Running them inside `WindowMod::update()` meant
//! they blocked MINECRAFT'S RENDER THREAD, producing the periodic 30-236 ms
//! spikes and the "browser freezes" stutter.
//!
//! This module moves ALL of that blocking window enumeration onto a DEDICATED
//! background thread. The render thread never enumerates windows anymore:
//!   * once in a while it publishes a cheap SNAPSHOT of what it currently tracks
//!     (`ScanInput`: tracked HWND+PID list, whether a launcher is open, the
//!     hidden-desktop handle) via `publish_input`,
//!   * the background thread does the heavy scans against the live OS window
//!     tree and publishes the RESULT (`ScanOutput`: HWNDs to adopt as
//!     launcher-children/popups, windows to re-hide) via the shared mutex,
//!   * the render thread then calls `apply` which registers those found windows
//!     using the existing (cheap, allocation-only) register functions.
//!
//! State mutation still happens ONLY on the render thread (inside `apply`), so
//! there are no data races on `WindowMod`; the background thread only ever
//! touches the OS and the two small shared snapshot structs.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows::Win32::System::StationsAndDesktops::HDESK;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetWindow, GetWindowLongW,
    GetWindowThreadProcessId, GetWindowRect, IsWindowVisible, GW_OWNER, GWL_STYLE, WS_CHILD,
    WS_VISIBLE,
};
use windows::Win32::System::StationsAndDesktops::EnumDesktopWindows;

use super::process::collect_descendant_pids_pub;

/// One tracked window as seen by the render thread, handed to the scanner.
#[derive(Clone, Copy)]
pub struct TrackedWin {
    pub hwnd: isize,
    pub pid: u32,
    pub is_popup: bool,
}

/// Snapshot the render thread publishes for the scanner to work against.
#[derive(Default, Clone)]
pub struct ScanInput {
    /// Every tracked toplevel/popup (hwnd, pid, is_popup).
    pub tracked: Vec<TrackedWin>,
    /// True if a real game launcher (Steam/Epic/…) is among the tracked windows;
    /// gates the expensive launcher-children scan.
    pub has_launcher: bool,
    /// Raw HDESK of the hidden desktop (0 if none), so the scanner can
    /// enumerate windows that live there.
    pub hidden_hdesk_raw: isize,
}

/// A popup candidate the scanner found (all in isize/raw form so it is Send).
#[derive(Clone, Copy)]
pub struct PopupCandidate {
    pub hwnd: isize,
    pub owner: isize,
}

/// Results the scanner publishes for the render thread to apply.
#[derive(Default, Clone)]
pub struct ScanOutput {
    /// New launcher-child windows found on the HIDDEN desktop (adopt + windowed).
    pub new_hidden_children: Vec<isize>,
    /// New launcher-child windows found on the VISIBLE desktop (adopt + park).
    pub new_visible_children: Vec<isize>,
    /// New popup windows (dropdowns/menus) with their owner HWND.
    pub new_popups: Vec<PopupCandidate>,
    /// Visible-desktop tracked windows that drifted on-screen and must be parked.
    pub redrift_visible: Vec<isize>,
}

static INPUT: OnceLock<Mutex<ScanInput>> = OnceLock::new();
static OUTPUT: OnceLock<Mutex<ScanOutput>> = OnceLock::new();
static STARTED: OnceLock<()> = OnceLock::new();

fn input() -> &'static Mutex<ScanInput> {
    INPUT.get_or_init(|| Mutex::new(ScanInput::default()))
}

fn output() -> &'static Mutex<ScanOutput> {
    OUTPUT.get_or_init(|| Mutex::new(ScanOutput::default()))
}

/// Publish the render thread's current tracked-window snapshot for the scanner.
/// Cheap: just clones a small Vec under a short-held lock.
pub fn publish_input(inp: ScanInput) {
    ensure_started();
    if let Ok(mut guard) = input().lock() {
        *guard = inp;
    }
}

/// Take (and clear) the scanner's latest results so the render thread can apply
/// them. Returns an empty ScanOutput when the scanner has nothing new.
pub fn take_output() -> ScanOutput {
    match output().lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(_) => ScanOutput::default(),
    }
}

/// Spawn the background scanner thread exactly once.
fn ensure_started() {
    STARTED.get_or_init(|| {
        std::thread::spawn(scanner_main);
    });
}

fn scanner_main() {
    loop {
        // Read the latest snapshot from the render thread.
        let inp = match input().lock() {
            Ok(g) => g.clone(),
            Err(_) => ScanInput::default(),
        };

        // Do the heavy, blocking OS scans OFF the render thread.
        let out = scan(&inp);

        // Publish results for the render thread to apply.
        if let Ok(mut guard) = output().lock() {
            // Merge rather than overwrite so results are not lost if the render
            // thread has not applied the previous batch yet.
            guard.new_hidden_children.extend(out.new_hidden_children);
            guard.new_visible_children.extend(out.new_visible_children);
            guard.new_popups.extend(out.new_popups);
            guard.redrift_visible.extend(out.redrift_visible);
            // Bound the queues so a render thread that stops applying can't make
            // them grow without limit.
            dedup_bound(&mut guard.new_hidden_children);
            dedup_bound(&mut guard.new_visible_children);
            guard.new_popups.truncate(64);
            dedup_bound(&mut guard.redrift_visible);
        }

        // Scan a few times per second — game-launch and menu detection tolerate
        // this, and it keeps the scanner's own CPU use negligible.
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn dedup_bound(v: &mut Vec<isize>) {
    let mut seen = HashSet::new();
    v.retain(|x| seen.insert(*x));
    if v.len() > 64 {
        v.truncate(64);
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

fn is_chromium_class(cls: &str) -> bool {
    cls == "Chrome_WidgetWin_1" || cls == "Chrome_WidgetWin_0"
}

/// Enumerate windows on the hidden desktop (or nothing if there is none).
fn for_each_hidden(hdesk_raw: isize, mut f: impl FnMut(HWND) -> bool) {
    if hdesk_raw == 0 {
        return;
    }
    let hdesk = HDESK(hdesk_raw as *mut _);

    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let f = &mut *(lparam.0 as *mut &mut dyn FnMut(HWND) -> bool);
        if f(hwnd) { TRUE } else { BOOL(0) }
    }
    let mut closure: &mut dyn FnMut(HWND) -> bool = &mut f;
    unsafe {
        let _ = EnumDesktopWindows(hdesk, Some(cb), LPARAM(&mut closure as *mut _ as isize));
    }
}

/// The heavy scan itself. Reads the OS window tree; touches no WindowMod state.
fn scan(inp: &ScanInput) -> ScanOutput {
    let mut out = ScanOutput::default();

    // Set of HWNDs we already track (never re-adopt).
    let known: HashSet<isize> = inp.tracked.iter().map(|t| t.hwnd).collect();

    // ---- Launcher-children scan (only when a launcher is actually open) -----
    if inp.has_launcher && !inp.tracked.is_empty() {
        // Build the descendant PID tree of every tracked non-popup window.
        let mut tree: HashSet<u32> = HashSet::new();
        for t in inp.tracked.iter().filter(|t| !t.is_popup) {
            if t.pid != 0 {
                for pid in collect_descendant_pids_pub(t.pid) {
                    tree.insert(pid);
                }
            }
        }

        // Hidden-desktop children (skip Chromium helper windows).
        for_each_hidden(inp.hidden_hdesk_raw, |hwnd| {
            let key = hwnd.0 as isize;
            if known.contains(&key) {
                return true;
            }
            let mut wpid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut wpid)) };
            if !tree.contains(&wpid) {
                return true;
            }
            let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
            if style & WS_CHILD.0 as i32 != 0 {
                return true;
            }
            let visible = (style & WS_VISIBLE.0 as i32) != 0
                || unsafe { IsWindowVisible(hwnd).as_bool() };
            if !visible {
                return true;
            }
            let cls = class_name(hwnd);
            if cls == "#32768" || is_chromium_class(&cls) {
                return true;
            }
            let mut rc = RECT::default();
            unsafe { let _ = GetClientRect(hwnd, &mut rc); }
            if rc.right - rc.left >= 100 && rc.bottom - rc.top >= 100 {
                out.new_hidden_children.push(key);
            }
            true
        });

        // Visible-desktop children (games a broker launched onto the real
        // desktop). Skip Chromium helper windows here too.
        {
            struct S<'a> {
                tree: &'a HashSet<u32>,
                known: &'a HashSet<isize>,
                found: Vec<isize>,
            }
            unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
                let s = &mut *(lparam.0 as *mut S);
                let key = hwnd.0 as isize;
                if s.known.contains(&key) {
                    return TRUE;
                }
                let mut wpid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut wpid));
                if !s.tree.contains(&wpid) {
                    return TRUE;
                }
                let style = GetWindowLongW(hwnd, GWL_STYLE);
                if style & WS_CHILD.0 as i32 != 0 {
                    return TRUE;
                }
                if !IsWindowVisible(hwnd).as_bool() {
                    return TRUE;
                }
                let mut cbuf = [0u16; 64];
                let clen = GetClassNameW(hwnd, &mut cbuf);
                if clen > 0 {
                    let cls = String::from_utf16_lossy(&cbuf[..clen as usize]);
                    if cls == "Chrome_WidgetWin_1" || cls == "Chrome_WidgetWin_0" {
                        return TRUE;
                    }
                }
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let w = rc.right - rc.left;
                let h = rc.bottom - rc.top;
                if w >= 200 && h >= 200 {
                    s.found.push(key);
                }
                TRUE
            }
            let mut s = S { tree: &tree, known: &known, found: Vec::new() };
            unsafe {
                let _ = EnumWindows(Some(cb), LPARAM(&mut s as *mut _ as isize));
            }
            out.new_visible_children = s.found;
        }
    }

    // ---- Popup scan (dropdowns/menus of tracked windows) --------------------
    if !inp.tracked.is_empty() {
        // Owner PIDs (only non-popup tracked windows can own a popup).
        let owner_pids: HashSet<u32> = inp
            .tracked
            .iter()
            .filter(|t| !t.is_popup)
            .map(|t| t.pid)
            .filter(|p| *p != 0)
            .collect();
        let owner_hwnds: HashSet<isize> = inp
            .tracked
            .iter()
            .filter(|t| !t.is_popup)
            .map(|t| t.hwnd)
            .collect();

        for_each_hidden(inp.hidden_hdesk_raw, |hwnd| {
            let key = hwnd.0 as isize;
            if known.contains(&key) {
                return true;
            }
            let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
            if style & WS_VISIBLE.0 as i32 == 0 || style & WS_CHILD.0 as i32 != 0 {
                return true;
            }
            let cls = class_name(hwnd);
            let is_menu = cls == "#32768";
            let is_popup_style = (style as u32 & 0x8000_0000) != 0; // WS_POPUP
            if !is_menu && !is_popup_style {
                return true;
            }
            let owner = unsafe { GetWindow(hwnd, GW_OWNER) }
                .unwrap_or(HWND(std::ptr::null_mut()));
            let owner_key = owner.0 as isize;
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };

            // Resolve which tracked owner this popup belongs to.
            let matched_owner = if !owner.0.is_null() && owner_hwnds.contains(&owner_key) {
                Some(owner_key)
            } else if is_menu && owner_pids.contains(&pid) {
                // Classic menu with null owner: attribute to a tracked window of
                // the same PID.
                inp.tracked
                    .iter()
                    .find(|t| !t.is_popup && t.pid == pid)
                    .map(|t| t.hwnd)
            } else {
                None
            };
            let Some(owner_hwnd) = matched_owner else {
                return true;
            };

            let mut rc = RECT::default();
            unsafe { let _ = GetClientRect(hwnd, &mut rc); }
            if rc.right - rc.left >= 4 && rc.bottom - rc.top >= 4 {
                out.new_popups.push(PopupCandidate { hwnd: key, owner: owner_hwnd });
            }
            true
        });
    }

    // ---- Off-screen re-hide scan (visible-desktop tracked windows) ----------
    // Build the set of HWNDs currently on the hidden desktop so we know which
    // tracked windows are on the VISIBLE desktop and might have drifted.
    let mut hidden_set: HashSet<isize> = HashSet::new();
    for_each_hidden(inp.hidden_hdesk_raw, |hwnd| {
        hidden_set.insert(hwnd.0 as isize);
        true
    });
    for t in inp.tracked.iter().filter(|t| !t.is_popup) {
        if hidden_set.contains(&t.hwnd) {
            continue; // on hidden desktop → already invisible
        }
        let hwnd = HWND(t.hwnd as *mut _);
        let mut rc = RECT::default();
        let drifted = unsafe {
            GetWindowRect(hwnd, &mut rc).is_ok() && (rc.left > -20000 || rc.top > -20000)
        };
        if drifted {
            out.redrift_visible.push(t.hwnd);
        }
    }

    out
}
