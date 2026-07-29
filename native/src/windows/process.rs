use std::collections::HashSet;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::System::StationsAndDesktops::{
    CreateDesktopW, EnumDesktopWindows, HDESK, DESKTOP_CREATEWINDOW, DESKTOP_WRITEOBJECTS,
    DESKTOP_READOBJECTS, DESKTOP_CREATEMENU, DESKTOP_HOOKCONTROL, DESKTOP_ENUMERATE,
    DESKTOP_SWITCHDESKTOP,
};





use windows::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcessId, STARTUPINFOW, STARTUPINFOW_FLAGS, PROCESS_INFORMATION,
};

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{SW_SHOWNORMAL, WS_EX_LAYERED};


use super::apps::{find_app, DesktopApp};
use super::capture::register_external_hwnd;


use super::state::{PendingLaunch, WindowMod};

pub fn spawn_app(state: &mut WindowMod, app_id: &str) -> bool {
    eprintln!("[windowmod] spawn_app called for app_id='{}'", app_id);
    let app = find_app(&state.desktop_apps, app_id).cloned();
    let Some(app) = app else {
        eprintln!("[windowmod]   app not found in desktop_apps (len={})", state.desktop_apps.len());
        return false;
    };
    eprintln!("[windowmod]   found app, name={:?} exec={:?}", app.name, app.exec);
    spawn_desktop_app(state, &app)
}

pub fn spawn_desktop_app(state: &mut WindowMod, app: &DesktopApp) -> bool {
    let Some(exec) = &app.exec else {
        eprintln!("[windowmod] spawn_desktop_app: no exec path for app_id='{}'", app.app_id);
        return false;
    };

    if app.exec_terminal {
        eprintln!("[windowmod] spawn_desktop_app: terminal app, exec={:?}", exec);
        let terminal = state.preferred_terminal.clone();
        if terminal.is_empty() {
            eprintln!("[windowmod]   preferred_terminal is empty, cannot launch terminal app");
            return false;
        }
        return spawn_executable(state, &terminal, &["/c", "start", "", exec], None, &app);
    }

    // Build the argument list from the shortcut's stored arguments (e.g.
    // Discord's `--processStart Discord.exe`). We split on whitespace, honouring
    // simple double-quoted groups, which covers the vast majority of real
    // shortcut command lines.
    let mut arg_storage: Vec<String> = match &app.exec_args {
        Some(a) if !a.trim().is_empty() => split_args(a),
        _ => Vec::new(),
    };

    // CHROMIUM/ELECTRON anti-FREEZE flags.
    //
    // Apps like Opera, Chrome, Discord, VS Code render through Chromium. When
    // their window lives on our HIDDEN desktop, Chromium's occlusion detection
    // decides the window is not visible and FREEZES its rendering to save power
    // — the tab stops painting, so PrintWindow keeps returning the SAME stale
    // frame and the window "hangs" after a while even though the app is alive.
    // This is the real cause of "Opera works, then freezes; Discord too".
    //
    // The documented way to stop that is to launch the browser with the flags
    // that disable occlusion calculation and renderer/timer backgrounding. They
    // are Chromium-specific, so we only add them when the target exe looks like
    // a Chromium/Electron app; other programs would ignore (or reject) them.
    if is_chromium_executable(exec) {
        for flag in [
            "--disable-features=CalculateNativeWinOcclusion",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            "--disable-background-timer-throttling",
        ] {
            // Avoid duplicating a flag the shortcut already carries.
            if !arg_storage.iter().any(|a| a.starts_with(flag.split('=').next().unwrap_or(flag))) {
                arg_storage.push(flag.to_string());
            }
        }
        eprintln!(
            "[windowmod] spawn_desktop_app: Chromium app detected, added anti-freeze flags for {:?}",
            exec,
        );
    }

    let args: Vec<&str> = arg_storage.iter().map(|s| s.as_str()).collect();
    let work_dir = app.working_dir.as_deref().filter(|d| !d.is_empty());


    eprintln!(
        "[windowmod] spawn_desktop_app: launching exec={:?} args={:?} work_dir={:?}",
        exec, args, work_dir,
    );
    // We resolve shortcuts to their real .exe target (see shortcut.rs), so we
    // launch the executable DIRECTLY with CreateProcessW. This inherits
    // STARTUPINFO.lpDesktop (the hidden desktop) and never spawns a console
    // window — unlike the old `cmd /c start` path, which both leaked a console
    // and let apps land on the visible desktop.
    spawn_executable(state, exec, &args, work_dir, app)
}

/// Split a shortcut command-line string into individual arguments, honouring
/// double-quoted groups. Good enough for real-world shortcut arguments.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut have_token = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                have_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if have_token {
                    out.push(std::mem::take(&mut cur));
                    have_token = false;
                }
            }
            c => {
                cur.push(c);
                have_token = true;
            }
        }
    }
    if have_token {
        out.push(cur);
    }
    out
}



/// Collect `root_pid` and ALL of its descendant process IDs (children,
/// grandchildren, …) by walking the system process snapshot. Used so a launcher
/// process (TLauncher, Steam, an installer, an Electron bootstrapper) that
/// spawns the real application as a SEPARATE child process still has that
/// child's window found and adopted into the same toplevel — which is what makes
/// "TLauncher launches Minecraft, and the window item turns into Minecraft"
/// work, and in reverse.
fn collect_descendant_pids(root_pid: u32) -> HashSet<u32> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut result: HashSet<u32> = HashSet::new();
    result.insert(root_pid);
    if root_pid == 0 {
        return result;
    }

    // Build a parent→pid edge list from one snapshot, then expand transitively.
    let mut edges: Vec<(u32, u32)> = Vec::new(); // (parent_pid, pid)
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return result;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                edges.push((entry.th32ParentProcessID, entry.th32ProcessID));
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }

    // Transitive closure: keep adding pids whose parent is already in the set.
    // Bounded iterations guard against pathological/cyclic parent values (PID
    // reuse can make a process appear to be its own ancestor).
    for _ in 0..64 {
        let before = result.len();
        for &(parent, pid) in &edges {
            if result.contains(&parent) {
                result.insert(pid);
            }
        }
        if result.len() == before {
            break;
        }
    }
    result
}

/// Find the main top-level window on the HIDDEN desktop owned by ANY pid in
/// `pids` (the launch's process tree). Mirrors `find_pid_window_hidden` but
/// matches a whole process tree, so a launcher's child app is also found.
fn find_tree_window_hidden(pids: &HashSet<u32>) -> Option<HWND> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowLongW, GetWindowThreadProcessId, GWL_STYLE, WS_CHILD,
    };

    let mut found: Option<HWND> = None;
    for_each_hidden_desktop_window(|hwnd| {
        let mut wpid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut wpid)) };
        if !pids.contains(&wpid) {
            return true;
        }
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
        if style & WS_CHILD.0 as i32 != 0 {
            return true;
        }
        let mut rc = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        if rc.right - rc.left >= 10 && rc.bottom - rc.top >= 10 {
            found = Some(hwnd);
            return false;
        }
        true
    });
    found
}

/// Find the main top-level window on the VISIBLE desktop owned by ANY pid in
/// `pids`. Mirrors `capture::find_main_window_for_pid` but matches a whole
/// process tree.
fn find_tree_window_visible(pids: &HashSet<u32>) -> Option<HWND> {
    use windows::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClientRect, GetWindowLongW, GetWindowThreadProcessId, GWL_STYLE,
        IsWindowVisible, WS_CHILD,
    };

    struct Search<'a> {
        pids: &'a HashSet<u32>,
        found: Option<HWND>,
    }

    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let s = &mut *(lparam.0 as *mut Search);
        let mut wpid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut wpid));
        if !s.pids.contains(&wpid) {
            return TRUE;
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        if style & WS_CHILD.0 as i32 != 0 {
            return TRUE;
        }
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        if rc.right - rc.left >= 10 && rc.bottom - rc.top >= 10 {
            s.found = Some(hwnd);
            return BOOL(0);
        }
        TRUE
    }

    let mut search = Search { pids, found: None };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}

fn hint_from_path(path: &str) -> String {

    path.split(&['/', '\\'])
        .last()
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// Heuristically decide whether `exec` is a Chromium/Electron application, so we
/// can pass it the anti-freeze command-line flags (occlusion + backgrounding
/// disable). We match on the executable's file name against a list of common
/// Chromium-based browsers and Electron apps. This is deliberately conservative:
/// the flags are only added for recognised apps, never for arbitrary programs
/// that might reject unknown switches.
fn is_chromium_executable(exec: &str) -> bool {
    let name = hint_from_path(exec); // lowercased exe stem, e.g. "opera"
    matches!(
        name.as_str(),
        "opera"
            | "opera_gx"
            | "operagx"
            | "launcher"      // Opera GX/One sometimes launch via launcher.exe
            | "chrome"
            | "msedge"
            | "edge"
            | "brave"
            | "vivaldi"
            | "yandex"
            | "browser"       // Yandex Browser's browser.exe
            | "discord"
            | "discordptb"
            | "discordcanary"
            | "slack"
            | "code"          // VS Code
            | "code - insiders"
            | "spotify"
            | "whatsapp"
            | "teams"
            | "ms-teams"
            | "chromium"
            | "thorium"
            | "electron"
            // Steam's client UI is CEF (Chromium Embedded Framework). On the
            // hidden desktop its renderer hits the SAME occlusion-freeze as
            // Discord/Opera did — the store/library stops painting and the
            // window "lags"/goes stale. The Steam client accepts Chromium
            // switches on its command line, so add the anti-freeze flags for it.
            | "steam"
            | "steamwebhelper"
            // Other CEF/Electron game launchers with the same freeze behaviour.
            | "epicgameslauncher"
            | "galaxyclient"   // GOG Galaxy
            | "battle.net"
            | "eadesktop"      // EA App
    )
}



/// Name of the hidden desktop we launch all moded apps onto. Apps on this
/// desktop never appear on the user's visible desktop, yet their windows are
/// "active" within that desktop so they keep rendering and (hopefully) accept
/// input — unlike an off-screen window on the visible desktop.
const HIDDEN_DESKTOP_NAME: &str = "WindowModDesktop";

/// Created once on first launch. Stored as the wide name (NUL-terminated) so we
/// can hand it to STARTUPINFOW.lpDesktop. Also stores the HDESK (as isize) so
/// we can enumerate windows on that desktop with EnumDesktopWindows.
static HIDDEN_DESKTOP: OnceLock<(Vec<u16>, isize)> = OnceLock::new();

fn hidden_desktop() -> &'static (Vec<u16>, isize) {
    HIDDEN_DESKTOP.get_or_init(|| {
        let name_wide: Vec<u16> = OsStr::new(HIDDEN_DESKTOP_NAME)
            .encode_wide()
            .chain([0])
            .collect();

        // CreateDesktopW takes the desired access as a raw u32.
        let access: u32 = DESKTOP_CREATEWINDOW.0
            | DESKTOP_WRITEOBJECTS.0
            | DESKTOP_READOBJECTS.0
            | DESKTOP_CREATEMENU.0
            | DESKTOP_HOOKCONTROL.0
            | DESKTOP_ENUMERATE.0
            | DESKTOP_SWITCHDESKTOP.0;

        let hdesk = unsafe {
            CreateDesktopW(
                PCWSTR(name_wide.as_ptr()),
                PCWSTR::null(),
                None,
                Default::default(),
                access,
                None::<*const SECURITY_ATTRIBUTES>,
            )
        };

        match hdesk {
            Ok(h) => {
                let raw = h.0 as isize;
                // Keep the desktop alive for the whole session: we simply never
                // call CloseDesktop. (HDESK is Copy, so there is nothing to
                // explicitly leak/forget.)
                let _ = h;

                eprintln!("[windowmod] Created hidden desktop '{}' (hdesk={raw:#x})", HIDDEN_DESKTOP_NAME);
                (name_wide, raw)
            }
            Err(e) => {
                eprintln!("[windowmod] CreateDesktopW failed: {e} — falling back to default desktop");
                (Vec::new(), 0)
            }
        }
    })
}

/// NUL-terminated wide desktop name for STARTUPINFOW.lpDesktop, or None if the
/// hidden desktop could not be created (fall back to inheriting our desktop).
pub(crate) fn hidden_desktop_name_wide() -> Option<&'static [u16]> {
    let (name, raw) = hidden_desktop();
    if *raw == 0 || name.is_empty() {
        None
    } else {
        Some(name.as_slice())
    }
}

/// Ensure the hidden desktop exists (idempotent). Returns true if available.
pub(crate) fn ensure_hidden_desktop() -> bool {
    hidden_desktop_name_wide().is_some()
}


fn hidden_hdesk() -> Option<HDESK> {
    let (_, raw) = hidden_desktop();
    if *raw == 0 {
        None
    } else {
        Some(HDESK(*raw as *mut _))
    }
}

/// Bind the CALLING thread to the hidden desktop via `SetThreadDesktop`.
///
/// This is REQUIRED for Windows Graphics Capture (WGC) to work on our windows.
/// `IGraphicsCaptureItemInterop::CreateForWindow` fails with 0x80070057
/// (E_INVALIDARG) when the target window lives on a DIFFERENT desktop than the
/// calling thread — which is exactly our situation: the capture threads run on
/// the process's default desktop while every captured app window lives on the
/// hidden `WindowModDesktop`. The log confirmed this: WGC init failed for EVERY
/// window, so all of them fell back to the slow GDI PrintWindow path (black /
/// stale frames, 10-19 ms each — the "everything lags and hangs" symptom).
///
/// Calling `SetThreadDesktop(hidden)` at the top of a capture thread moves that
/// thread onto the hidden desktop, so `CreateForWindow` sees the window on its
/// own desktop and succeeds. It must be called BEFORE any windows/COM objects
/// are created on the thread (SetThreadDesktop fails if the thread already has
/// windows), which is why the capture thread calls it first thing.
///
/// Returns true on success (or if there is no hidden desktop — nothing to do).
pub fn bind_thread_to_hidden_desktop() -> bool {
    use windows::Win32::System::StationsAndDesktops::SetThreadDesktop;
    let Some(hdesk) = hidden_hdesk() else {
        return true; // no hidden desktop → threads stay on default, which is fine
    };
    unsafe { SetThreadDesktop(hdesk).is_ok() }
}

/// Public wrapper so the background scanner thread can build process trees
/// without duplicating the snapshot walk.
pub fn collect_descendant_pids_pub(root_pid: u32) -> HashSet<u32> {
    collect_descendant_pids(root_pid)
}

/// Raw HDESK of the hidden desktop as an isize (0 if none) for the scanner.
pub fn hidden_hdesk_raw() -> isize {
    let (_, raw) = hidden_desktop();
    *raw
}

/// Build the cheap ScanInput snapshot from the current tracked windows and
/// hand it to the background scanner. Called on the render thread; it only
/// reads each tracked window's PID (a fast, non-blocking call) — no window
/// enumeration happens here.
pub fn publish_scan_input(state: &WindowMod) {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let mut tracked = Vec::with_capacity(state.toplevels.len());
    let mut has_launcher = false;
    for t in state.toplevels.iter() {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(t.hwnd, Some(&mut pid)) };
        tracked.push(super::scanner::TrackedWin {
            hwnd: t.hwnd.0 as isize,
            pid,
            is_popup: t.is_popup,
        });
    }
    // Decide whether a launcher is open (gates the expensive children scan).
    // Reuse the existing image-name check.
    if should_scan_launcher_children(state) {
        has_launcher = true;
    }

    super::scanner::publish_input(super::scanner::ScanInput {
        tracked,
        has_launcher,
        hidden_hdesk_raw: hidden_hdesk_raw(),
    });
}

/// Apply the background scanner's results on the render thread: register the
/// windows it found (launcher-children, popups) and re-park drifted visible
/// windows. Each register call is cheap (allocation only, no window
/// enumeration), so this never blocks the render thread.
pub fn apply_scan_output(state: &mut WindowMod) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };

    let out = super::scanner::take_output();

    // Currently-tracked HWNDs, so we never double-register something the render
    // thread already adopted between scans.
    let known: HashSet<isize> = state.toplevels.iter().map(|t| t.hwnd.0 as isize).collect();

    // Hidden-desktop launcher children → adopt + force windowed.
    for key in out.new_hidden_children {
        if known.contains(&key) {
            continue;
        }
        let hwnd = HWND(key as *mut _);
        if register_external_hwnd(state, hwnd, "game".to_string()).is_some() {
            make_game_window(hwnd);
        }
    }

    // Visible-desktop launcher children → adopt + windowed + park off-screen.
    for key in out.new_visible_children {
        if state.toplevels.iter().any(|t| t.hwnd.0 as isize == key) {
            continue;
        }
        let hwnd = HWND(key as *mut _);
        if register_external_hwnd(state, hwnd, "game".to_string()).is_some() {
            make_game_window(hwnd);
            park_offscreen(hwnd);
        }
    }

    // Popups → register inside their owner (needs the owner's root surface).
    for cand in out.new_popups {
        if state.toplevels.iter().any(|t| t.hwnd.0 as isize == cand.hwnd) {
            continue;
        }
        let owner_hwnd = HWND(cand.owner as *mut _);
        // Find the owner toplevel's root surface pointer.
        let owner_surface = state
            .toplevels
            .iter()
            .find(|t| !t.is_popup && t.hwnd == owner_hwnd)
            .and_then(|t| {
                let tl_ptr = super::state::ptr_of_ref(&**t);
                state.surface_for_toplevel(tl_ptr)
            });
        let Some(owner_surface_ptr) = owner_surface else {
            continue;
        };
        let popup_hwnd = HWND(cand.hwnd as *mut _);
        if super::capture::register_popup_hwnd(state, popup_hwnd, owner_hwnd, owner_surface_ptr)
            .is_some()
        {
            make_compositor_window(popup_hwnd);
        }
    }

    // Re-park visible-desktop windows that drifted back on-screen.
    for key in out.redrift_visible {
        let hwnd = HWND(key as *mut _);
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                -32000,
                -32000,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
            );
        }
    }

    // Also keep popup offsets fresh (cheap, touches only tracked popups) — this
    // was previously the first half of poll_popup_windows and does not enumerate
    // the desktop, so it is fine on the render thread.
    refresh_tracked_popup_offsets(state);
}

/// Refresh the in-owner offset of popups we already track (owner may have
/// moved/resized). Does NOT enumerate the desktop — only walks tracked popups.
fn refresh_tracked_popup_offsets(state: &mut WindowMod) {
    let popups: Vec<(i64, HWND, HWND)> = state
        .toplevels
        .iter()
        .filter(|t| t.is_popup)
        .map(|t| (super::state::ptr_of_ref(&**t), t.hwnd, t.owner_hwnd))
        .collect();
    for (tl_ptr, popup_hwnd, owner_hwnd) in popups {
        if !super::capture::hwnd_alive(popup_hwnd) || !super::capture::hwnd_alive(owner_hwnd) {
            continue;
        }
        let (xoff, yoff) = super::capture::refresh_popup_offset(popup_hwnd, owner_hwnd);
        for s in state.surfaces.iter_mut() {
            if s.toplevel_ptr == tl_ptr && (s.xoff != xoff || s.yoff != yoff) {
                s.xoff = xoff;
                s.yoff = yoff;
                s.buffer_dirty = true;
            }
        }
    }
}



/// True if `hwnd` currently lives on our HIDDEN desktop. Windows we adopted from
/// the VISIBLE desktop (UWP brokered apps parked off-screen) return false.
///
/// Used by input routing: driving UI Automation Invoke/SetFocus on a window that
/// shares the user's VISIBLE desktop steals the global foreground from Minecraft
/// (which then fires ESC). For such windows we must rely on PostMessage only.
pub fn is_on_hidden_desktop(hwnd: HWND) -> bool {
    let target = hwnd.0 as isize;
    let mut found = false;
    for_each_hidden_desktop_window(|h| {
        if h.0 as isize == target {
            found = true;
            false // stop
        } else {
            true
        }
    });
    found
}

/// Enumerate top-level windows on the hidden desktop (EnumWindows only sees the
/// caller thread's desktop, so it can't see them). Calls `f(hwnd)` for each;
/// `f` returns false to stop early.
fn for_each_hidden_desktop_window(mut f: impl FnMut(HWND) -> bool) {
    use windows::Win32::Foundation::{BOOL, LPARAM, TRUE};

    let Some(hdesk) = hidden_hdesk() else { return };

    // Trampoline: LPARAM carries a pointer to the boxed closure.
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let f = &mut *(lparam.0 as *mut &mut dyn FnMut(HWND) -> bool);
        if f(hwnd) { TRUE } else { BOOL(0) }
    }

    let mut closure: &mut dyn FnMut(HWND) -> bool = &mut f;
    unsafe {
        let _ = EnumDesktopWindows(
            hdesk,
            Some(cb),
            LPARAM(&mut closure as *mut _ as isize),
        );
    }
}



fn dump_all_windows() {
    use windows::Win32::Foundation::{BOOL, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    };

    struct Ctx {
        count: u32,
    }
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        ctx.count += 1;
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let len = GetWindowTextLengthW(hwnd);
        let title = if len > 0 {
            let mut buf = vec![0u16; (len + 1) as usize];
            let read = GetWindowTextW(hwnd, &mut buf);
            String::from_utf16_lossy(&buf[..read as usize])
        } else {
            String::new()
        };
        let truncated = if title.len() > 80 {
            format!("{}...", &title[..80])
        } else {
            title
        };
        eprintln!(
            "[windowmod]   HWND {:?} PID {} title='{}'",
            hwnd, pid, truncated
        );
        TRUE
    }

    let mut ctx = Ctx { count: 0 };
    eprintln!("[windowmod] --- Window dump (all top-level HWNDs) ---");
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut ctx as *mut _ as isize));
    }
    eprintln!("[windowmod] --- End dump ({} total) ---", ctx.count);
}

fn snapshot_hwnds() -> HashSet<isize> {
    use windows::Win32::Foundation::{BOOL, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

    let mut set = HashSet::new();
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let set = &mut *(lparam.0 as *mut HashSet<isize>);
        set.insert(hwnd.0 as isize);
        TRUE
    }
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut set as *mut _ as isize));
    }
    set
}

/// Find a brand-new top-level window (present now but not in `snapshot`) on our
/// own desktop, skipping our own process and child windows.
fn find_new_window_safe(snapshot: &HashSet<isize>) -> Option<HWND> {
    use windows::Win32::Foundation::{BOOL, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId};

    struct Search<'a> {
        snapshot: &'a HashSet<isize>,
        own_pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == search.own_pid {
            return TRUE;
        }
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GWL_STYLE, WS_CHILD};
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        if style & WS_CHILD.0 as i32 != 0 {
            return TRUE;
        }
        if !search.snapshot.contains(&(hwnd.0 as isize)) {
            search.found = Some(hwnd);
            return BOOL(0);
        }
        TRUE
    }

    let mut search = Search {
        snapshot,
        own_pid: unsafe { GetCurrentProcessId() },
        found: None,
    };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}


/// Find the main top-level window owned by `pid` on the hidden desktop.
///
/// Skips child windows AND tiny helper windows (Chromium/Notepad spawn 1x1
/// message/owner windows that share the PID). We require a reasonably sized
/// window so register_external_hwnd doesn't reject it for being too small.
fn find_pid_window_hidden(pid: u32) -> Option<HWND> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowLongW, GetWindowThreadProcessId, GWL_STYLE, WS_CHILD,
    };

    let mut found: Option<HWND> = None;
    for_each_hidden_desktop_window(|hwnd| {
        let mut wpid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut wpid)) };
        if wpid != pid {
            return true;
        }
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
        if style & WS_CHILD.0 as i32 != 0 {
            return true;
        }
        // Require a real, non-tiny client area so we register the actual app
        // window, not a 1x1 helper/message window with the same PID.
        let mut rc = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        if w >= 10 && h >= 10 {
            found = Some(hwnd);
            return false; // stop
        }
        true
    });
    found
}

/// Snapshot of all top-level window handles currently on the HIDDEN desktop.
/// Used to detect a brand-new window appearing there after a launch.
fn snapshot_hidden_hwnds() -> HashSet<isize> {
    let mut set = HashSet::new();
    for_each_hidden_desktop_window(|hwnd| {
        set.insert(hwnd.0 as isize);
        true
    });
    set
}

/// Find any brand-new, real top-level window on the HIDDEN desktop that was not
/// present in `before`. Because the hidden desktop only ever contains windows
/// WE launched there, the first new real window after a launch is almost
/// certainly the app we just started — even if it forked into a different PID
/// (Chromium browsers) or has an unexpected title.
fn find_any_new_hidden_window(before: &HashSet<isize>) -> Option<HWND> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowLongW, GWL_STYLE, WS_CHILD,
    };

    let mut found: Option<HWND> = None;
    for_each_hidden_desktop_window(|hwnd| {
        if before.contains(&(hwnd.0 as isize)) {
            return true;
        }
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
        if style & WS_CHILD.0 as i32 != 0 {
            return true;
        }
        // Skip untitled helper windows; real app windows have a title.
        // win_title_len is non-blocking (no WM_GETTEXT) — see helper docs.
        let len = win_title_len(hwnd);
        if len <= 0 {
            return true;
        }

        let mut rc = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        if rc.right - rc.left >= 10 && rc.bottom - rc.top >= 10 {
            found = Some(hwnd);
            return false; // stop
        }
        true
    });
    found
}



fn spawn_executable(
    state: &mut WindowMod,
    program: &str,
    args: &[&str],
    work_dir: Option<&str>,
    app: &DesktopApp,
) -> bool {
    let program_wide: Vec<u16> = OsStr::new(program).encode_wide().chain([0]).collect();

    // Wide, NUL-terminated working directory for CreateProcessW's
    // lpCurrentDirectory. Many apps (Discord's Update.exe, portable apps) only
    // start correctly when launched from their own directory.
    let work_dir_wide: Option<Vec<u16>> = work_dir
        .filter(|d| !d.is_empty())
        .map(|d| OsStr::new(d).encode_wide().chain([0]).collect());



    let mut cmdline_wide: Option<Vec<u16>> = None;
    if !args.is_empty() {
        let mut cmdline = String::new();
        cmdline.push('"');
        cmdline.push_str(program);
        cmdline.push('"');
        for arg in args {
            cmdline.push(' ');
            if arg.contains(' ') {
                cmdline.push('"');
                cmdline.push_str(arg);
                cmdline.push('"');
            } else {
                cmdline.push_str(arg);
            }
        }
        cmdline_wide = Some(OsStr::new(&cmdline).encode_wide().chain([0]).collect());
    }

    // Snapshot windows BEFORE launch. On the hidden desktop this lets us detect
    // the brand-new window even if the app forks into a different PID or has an
    // unexpected title (Chromium browsers). If the hidden desktop is
    // unavailable we fall back to the visible-desktop snapshot.
    let on_hidden_now = hidden_desktop_name_wide().is_some();
    let snapshot = if on_hidden_now {
        snapshot_hidden_hwnds()
    } else {
        snapshot_hwnds()
    };
    eprintln!("[windowmod] Snapshotted {} existing HWNDs (hidden={})", snapshot.len(), on_hidden_now);

    // Launch the app onto a SEPARATE, HIDDEN desktop (lpDesktop). This is the
    // key to making real apps (especially Chromium-based browsers) work:
    //   * The window never appears on the user's VISIBLE desktop — a desktop is
    //     a real isolation boundary, unlike parking a window offscreen, which
    //     the app can (and Chromium does) undo by repositioning itself.
    //   * The window is genuinely shown (SW_SHOWNORMAL, not SW_HIDE) on that
    //     hidden desktop, so it gets a real size, renders normally, and accepts
    //     synthesized input like any active window.
    // If the hidden desktop could not be created we fall back to inheriting our
    // own desktop (lpDesktop = NULL).
    let desktop_name = hidden_desktop_name_wide();
    let lp_desktop = match desktop_name {
        Some(name) => PWSTR(name.as_ptr() as *mut u16),
        None => PWSTR::null(),
    };

    let mut pi = PROCESS_INFORMATION::default();
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        lpDesktop: lp_desktop,
        dwFlags: STARTUPINFOW_FLAGS(0x00000001),
        // SW_SHOWNORMAL (not SW_HIDE): the desktop itself is hidden, so the
        // window stays invisible to the user, but it gets a real size and
        // becomes input-capable. SW_HIDE here left windows zero-sized.
        wShowWindow: SW_SHOWNORMAL.0 as u16,
        ..Default::default()
    };




    let pid = unsafe {
        match CreateProcessW(
            PCWSTR(program_wide.as_ptr()),
            if let Some(ref mut cmd) = cmdline_wide {
                PWSTR(cmd.as_mut_ptr())
            } else {
                PWSTR::null()
            },
            None::<*const SECURITY_ATTRIBUTES>,
            None::<*const SECURITY_ATTRIBUTES>,
            false,
            Default::default(),
            None::<*const std::ffi::c_void>,
            // lpCurrentDirectory: launch from the shortcut's working directory
            // when it has one, otherwise inherit ours (null).
            match &work_dir_wide {
                Some(w) => PCWSTR(w.as_ptr()),
                None => PCWSTR::null(),
            },
            &si as *const STARTUPINFOW,
            &mut pi as *mut PROCESS_INFORMATION,
        ) {
            Ok(()) => {
                let pid = pi.dwProcessId;
                let _ = CloseHandle(pi.hThread);
                let _ = CloseHandle(pi.hProcess);
                eprintln!("[windowmod] CreateProcessW started PID {pid} for {program}");
                pid
            }
            Err(e) => {
                eprintln!("[windowmod] CreateProcessW failed for {program}: {e}");
                return fallback_shellexec(state, program, args, app, &snapshot);
            }
        }
    };


    let name_hint = app.name.as_deref().unwrap_or("").to_lowercase();
    let path_hint = hint_from_path(program);
    eprintln!(
        "[windowmod] CreateProcessW PID {pid}, pushed pending (name='{name_hint}' path='{path_hint}')"
    );

    // Remember this as a process WE launched, so we can terminate it (and its
    // descendants) when Minecraft shuts down. Only PIDs added here are killed;
    // anything already running before the user launched it via the mod is never
    // recorded and therefore left untouched.
    if pid != 0 {
        state.launched_pids.push(pid);
    }

    // Window detection happens in poll_pending_launches on the next frame(s)


    state.pending_launches.push(PendingLaunch {
        pid,
        app_id: app.app_id.clone(),
        attempts: 0,
        hint: name_hint.clone(),
        alt_hint: if name_hint != path_hint && !path_hint.is_empty() {
            Some(path_hint)
        } else {
            None
        },
        snapshot,
        relaunched: false,
        hwnd_found: None,
        hwnd_found_attempt: 0,
        rejected_hwnds: HashSet::new(),
        toplevel_ptr: None,
        root_pid: pid,
    });
    true
}

fn fallback_shellexec(

    state: &mut WindowMod,
    program: &str,
    args: &[&str],
    app: &DesktopApp,
    snapshot: &HashSet<isize>,
) -> bool {
    let op: Vec<u16> = OsStr::new("open").encode_wide().chain([0]).collect();
    let file: Vec<u16> = OsStr::new(program).encode_wide().chain([0]).collect();
    let params: Vec<u16> = if args.is_empty() {
        vec![0]
    } else {
        OsStr::new(&args.join(" "))
            .encode_wide()
            .chain([0])
            .collect()
    };

    unsafe {
        let result = ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if (result.0 as isize) <= 32 {
            eprintln!("[windowmod] ShellExecuteW failed for {program}");
            return false;
        }
    }

    let name_hint = app.name.as_deref().unwrap_or("").to_lowercase();
    let path_hint = hint_from_path(program);
    eprintln!(
        "[windowmod] ShellExecuteW started {program}, pushed pending (name='{name_hint}')"
    );

    state.pending_launches.push(PendingLaunch {
        pid: 0,
        app_id: app.app_id.clone(),
        attempts: 0,
        hint: name_hint.clone(),
        alt_hint: if name_hint != path_hint && !path_hint.is_empty() {
            Some(path_hint)
        } else {
            None
        },
        snapshot: snapshot.clone(),
        relaunched: false,
        hwnd_found: None,
        hwnd_found_attempt: 0,
        rejected_hwnds: HashSet::new(),
        toplevel_ptr: None,
        root_pid: 0,
    });
    true
}

fn relaunch_visible(

    state: &mut WindowMod,
    program: &str,
    args: &[&str],
    idx: usize,
) {
    use windows::core::PWSTR;

    let program_wide: Vec<u16> = OsStr::new(program).encode_wide().chain([0]).collect();

    let mut cmdline_wide: Option<Vec<u16>> = None;
    if !args.is_empty() {
        let mut cmdline = String::new();
        cmdline.push('"');
        cmdline.push_str(program);
        cmdline.push('"');
        for arg in args {
            cmdline.push(' ');
            if arg.contains(' ') {
                cmdline.push('"');
                cmdline.push_str(arg);
                cmdline.push('"');
            } else {
                cmdline.push_str(arg);
            }
        }
        cmdline_wide = Some(OsStr::new(&cmdline).encode_wide().chain([0]).collect());
    }

    let mut pi = PROCESS_INFORMATION::default();
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };

    let pid = unsafe {
        match CreateProcessW(
            PCWSTR(program_wide.as_ptr()),
            if let Some(ref mut cmd) = cmdline_wide {
                PWSTR(cmd.as_mut_ptr())
            } else {
                PWSTR::null()
            },
            None::<*const SECURITY_ATTRIBUTES>,
            None::<*const SECURITY_ATTRIBUTES>,
            false,
            Default::default(),
            None::<*const std::ffi::c_void>,
            PCWSTR::null(),
            &si as *const STARTUPINFOW,
            &mut pi as *mut PROCESS_INFORMATION,
        ) {
            Ok(()) => {
                let pid = pi.dwProcessId;
                let _ = CloseHandle(pi.hThread);
                let _ = CloseHandle(pi.hProcess);
                eprintln!("[windowmod] Relaunch (visible) PID {pid} for {program} (replacing pending entry)");
                pid
            }
            Err(e) => {
                eprintln!("[windowmod] Relaunch failed for {program}: {e}");
                return;
            }
        }
    };

    let fresh_snapshot = snapshot_hwnds();
    state.pending_launches[idx].pid = pid;
    state.pending_launches[idx].attempts = 0;
    state.pending_launches[idx].snapshot = fresh_snapshot;
    state.pending_launches[idx].relaunched = true;
}

pub fn poll_pending_launches(state: &mut WindowMod) {
    let mut i = 0;
    while i < state.pending_launches.len() {
        let attempts = state.pending_launches[i].attempts;

        // Drop unresolved launches that time out. A pending launch that never
        // resolves (e.g. an app that forks into another PID we can't match)
        // otherwise keeps scanning ALL top-level windows every poll, which makes
        // native update() spike to hundreds of ms and tanks FPS.
        //
        // The previous limit of 40 was too low: poll_pending_launches only runs
        // every 30 FRAMES, and heavy apps (Electron/Chromium browsers, large
        // installers, .NET apps) routinely take more than a second to create
        // their first real window — so they were dropped before their window
        // ever appeared, which is why "many apps did not open". 200 attempts
        // gives a generous window for slow starters while still bounding the
        // cost for launches that truly never resolve.
        if attempts >= 200 {

            eprintln!(
                "[windowmod] Timed out finding valid window for '{}' — dropping",
                state.pending_launches[i].app_id
            );
            state.pending_launches.swap_remove(i);
            continue;
        }

        if attempts > 0 && attempts % 60 == 0 {

            eprintln!(
                "[windowmod] Still searching for '{}' attempt {} (hint='{}' relaunched={})",
                state.pending_launches[i].app_id,
                attempts,
                state.pending_launches[i].hint,
                state.pending_launches[i].relaunched,
            );
        }

        // If we already registered a window for this launch, check whether it is
        // still alive. Apps like VS Code / Electron and many installers first
        // create a SPLASH/loading window, then DESTROY it and create a separate
        // MAIN window (often reusing the SAME process/PID). We register the
        // splash, but when it is destroyed `retain_toplevels` removes it and the
        // real main window — which appears moments later — would never be found
        // if we had already dropped this pending launch. So we keep the pending
        // launch alive and, while its registered window is still alive, SKIP the
        // expensive window scan entirely (just a cheap IsWindow check). Only when
        // that window dies do we resume searching, which then picks up the new
        // main window.
        if let Some(hwnd_val) = state.pending_launches[i].hwnd_found {
            let prev_hwnd = HWND(hwnd_val as *mut _);
            if super::capture::hwnd_alive(prev_hwnd) {
                // Registered window still alive — nothing to do this poll. Do NOT
                // increment attempts toward the timeout while a live window is
                // being tracked, so the launch is monitored for as long as the
                // window exists (catching a later splash→main swap).
                i += 1;
                continue;
            }
            eprintln!(
                "[windowmod] Previously found HWND {:?} for '{}' died — re-searching attempt {}",
                prev_hwnd, state.pending_launches[i].app_id, attempts,
            );
            state.pending_launches[i].hwnd_found = None;
            state.pending_launches[i].hwnd_found_attempt = 0;
            // Reset the attempt counter so the splash→main re-search gets a fresh
            // full timeout window rather than expiring immediately.
            state.pending_launches[i].attempts = 0;
        }


        let (pid, root_pid, _snapshot, hint, alt_hint, existing_ptr) = {
            let launch = &state.pending_launches[i];
            (
                launch.pid,
                launch.root_pid,
                &launch.snapshot,
                &launch.hint,
                &launch.alt_hint,
                launch.toplevel_ptr,
            )
        };

        // Build the process tree (root + all descendants). A launcher such as
        // TLauncher/Steam/an installer spawns the real app as a CHILD process
        // with a DIFFERENT pid; matching the whole tree lets us find that child's
        // window and adopt it into the SAME toplevel, so the window-item turns
        // from the launcher into the launched app (and back). We rebuild it each
        // poll because new children appear over time.
        let tree_pids = collect_descendant_pids(root_pid);

        // Resolve the target window. We try, in order:
        //   1) PROCESS-TREE match on the hidden desktop (covers launcher→child),
        //   2) exact-PID match on the hidden desktop,
        //   3) title-hint match on the hidden desktop,
        //   4) any brand-new hidden window,
        //   5) VISIBLE-desktop process-tree / PID / hint (brokered UWP, etc.).
        // The visible-desktop scans are expensive (EnumWindows over every
        // window), so they only run on a subset of attempts.
        let on_hidden = hidden_desktop_name_wide().is_some();
        let visible_probe = attempts <= 30 || (attempts > 30 && attempts % 20 == 0);

        let (hwnd, search_method): (Option<HWND>, &str) = if on_hidden {
            if let Some(h) = find_tree_window_hidden(&tree_pids) {
                (Some(h), "hidden-tree")
            } else if let Some(h) = (pid != 0).then(|| find_pid_window_hidden(pid)).flatten() {
                (Some(h), "hidden-pid")
            } else if let Some(h) = find_hidden_window_by_hint(hint, alt_hint) {
                (Some(h), "hidden-hint")
            } else if let Some(h) = find_any_new_hidden_window(_snapshot) {
                (Some(h), "hidden-new")
            } else if visible_probe {
                if let Some(h) = find_tree_window_visible(&tree_pids) {
                    (Some(h), "visible-tree")
                } else if let Some(h) =
                    (pid != 0).then(|| super::capture::find_main_window_for_pid(pid)).flatten()
                {
                    (Some(h), "visible-pid")
                } else {
                    (find_by_hint(hint, alt_hint), "visible-hint")
                }
            } else {
                (None, "none")
            }
        } else if let Some(h) =
            (pid != 0).then(|| super::capture::find_main_window_for_pid(pid)).flatten()
        {
            (Some(h), "pid")
        } else if let Some(h) = find_by_hint(hint, alt_hint) {
            (Some(h), "hint")
        } else {
            (
                find_new_window_safe(&state.pending_launches[i].snapshot),
                "snapshot",
            )
        };
        let _ = &_snapshot;

        if let Some(hwnd) = hwnd {
            if state.pending_launches[i].rejected_hwnds.contains(&(hwnd.0 as isize)) {
                i += 1;
                continue;
            }

            // SINGLE-INSTANCE apps — by HWND already tracked (Discord, many Electron apps, browsers with

            // an existing window): relaunching the app does NOT create a new
            // window — the freshly-spawned process hands off to the EXISTING
            // window and exits. The window we find is therefore one we ALREADY
            // track under another toplevel. Previously we rejected it

            // ("already registered") and then latched onto an ephemeral helper
            // window of the short-lived process, which died moments later and
            // left a blank/transparent window. Instead: if the found HWND is
            // already owned by an existing toplevel, ADOPT that toplevel for
            // this launch (so the player's item shows the already-running app)
            // and finish — there is no new window to wait for.
            if let Some(existing) = state
                .toplevels
                .iter()
                .find(|t| t.hwnd == hwnd && !t.is_popup)
            {
                let existing_ptr = super::state::ptr_of_ref(&**existing);
                eprintln!(
                    "[windowmod] poll_pending_launches: HWND {:?} already tracked (single-instance) — adopting existing toplevel ptr={} and finishing",
                    hwnd, existing_ptr,
                );
                state.pending_launches[i].hwnd_found = Some(hwnd.0 as isize);
                state.pending_launches[i].toplevel_ptr = Some(existing_ptr);
                // Drop the launch: the window already exists and is tracked, so
                // there is nothing more to find. Keeping it would let the
                // splash→main pin logic hold a second reference unnecessarily.
                state.pending_launches.swap_remove(i);
                continue;
            }

            let app_id = state.pending_launches[i].app_id.clone();
            let already_found = state.pending_launches[i].hwnd_found.is_some();
            if !already_found {

                eprintln!(
                    "[windowmod] poll_pending_launches found HWND {:?} for {} attempt {} (method={})",
                    hwnd, app_id, attempts, search_method
                );
                let on_visible = search_method.starts_with("visible-");

                // If this launch already created a toplevel earlier (its window
                // was replaced — splash→main, launcher→app), REUSE that same
                // toplevel pointer for the new HWND so the player's window-item
                // (which stores the pointer) seamlessly follows the new window.
                // Otherwise create a fresh toplevel.
                let reused = if let Some(ptr) = existing_ptr {
                    if super::capture::reassign_toplevel_hwnd(state, ptr, hwnd, app_id.clone()) {
                        Some(ptr)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let registered = reused.or_else(|| register_external_hwnd(state, hwnd, app_id));
                if let Some(ptr) = registered {
                    // Windows found on the VISIBLE desktop (UWP brokered apps,
                    // re-launched browsers) must be parked far off-screen so the
                    // user does not see them on their real desktop while we
                    // render them inside Minecraft. Windows on the hidden
                    // desktop are already invisible, so they only need normal
                    // compositor preparation.
                    if on_visible {
                        park_offscreen(hwnd);
                    } else {
                        make_compositor_window(hwnd);
                    }

                    let reg_attempts = state.pending_launches[i].attempts;
                    state.pending_launches[i].hwnd_found = Some(hwnd.0 as isize);
                    state.pending_launches[i].hwnd_found_attempt = reg_attempts;
                    // Remember the toplevel pointer so a later window swap reuses
                    // it (keeping the window-item valid across the swap).
                    state.pending_launches[i].toplevel_ptr = Some(ptr);
                    eprintln!(
                        "[windowmod]   -> registered OK ptr={}, toplevels={}, reused={}, attempts_at_reg={}",
                        ptr,
                        state.toplevels.len(),
                        reused.is_some(),
                        reg_attempts,
                    );
                } else {
                    eprintln!("[windowmod]   -> register_external_hwnd returned None! (retrying next frame)");
                    state.pending_launches[i].rejected_hwnds.insert(hwnd.0 as isize);
                }
            }
        }

        state.pending_launches[i].attempts += 1;


        // NOTE: we intentionally DO NOT drop the pending launch right after the
        // first successful registration anymore. Apps such as VS Code / Electron
        // and many installers replace an initial SPLASH window with a separate
        // MAIN window a moment later; if we dropped the launch immediately, the
        // splash would be removed by `retain_toplevels` and the real window would
        // never be adopted (this is the "window vanishes after the loading
        // screen" bug). Instead we keep the launch alive: the early `continue`
        // above makes the per-poll cost a single cheap `IsWindow` check while the
        // registered window stays alive (no expensive scan), and the launch is
        // finally dropped only by the 200-attempt timeout once it is no longer
        // tracking a live window.

        i += 1;
    }
}


fn make_compositor_window(hwnd: HWND) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, ShowWindow, GWL_STYLE,
            WS_MINIMIZE, WS_MAXIMIZE, SW_SHOWNORMAL,
        };

        eprintln!("[windowmod] Preparing HWND {:?} on hidden desktop", hwnd);

        // The window already lives on a SEPARATE, HIDDEN desktop, so it is never
        // visible to the user — we do NOT need to (and must not) move it
        // offscreen on the visible desktop. We only normalize its state so it
        // has a real client size and renders.

        // Clear any minimized/maximized state so the window has a normal,
        // capturable client area.
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        let new_style = style & !(WS_MINIMIZE.0 as i32 | WS_MAXIMIZE.0 as i32);
        if new_style != style {
            let _ = SetWindowLongW(hwnd, GWL_STYLE, new_style);
        }

        // Ensure it is shown (normally) on its hidden desktop so it keeps a
        // real size and renders. This is invisible to the user because the
        // desktop itself is not the active/visible one.
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
    }
}

/// Park a window that landed on the VISIBLE desktop far off-screen so the user
/// never sees it, while keeping it shown (so it keeps a real client size and
/// renders for capture). Used only for apps whose real process was launched on
/// the interactive desktop by a system broker (UWP) and therefore could not be
/// placed on the hidden desktop.
fn park_offscreen(hwnd: HWND) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos, ShowWindow, GWL_STYLE,
            SWP_NOACTIVATE, SWP_NOZORDER, SWP_NOSIZE, WS_MINIMIZE, WS_MAXIMIZE, SW_SHOWNOACTIVATE,
        };

        eprintln!("[windowmod] Parking VISIBLE-desktop HWND {:?} off-screen", hwnd);

        // Clear minimized/maximized so it has a normal, capturable client area.
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        let new_style = style & !(WS_MINIMIZE.0 as i32 | WS_MAXIMIZE.0 as i32);
        if new_style != style {
            let _ = SetWindowLongW(hwnd, GWL_STYLE, new_style);
        }

        // Show without activating (so we don't steal Minecraft's foreground),
        // then move it far off the virtual screen so it is never visible.
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            hwnd,
            None,
            -32000,
            -32000,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
        );
    }
}

pub fn ensure_windows_hidden(state: &mut WindowMod) {
    // Apps on the hidden desktop are never visible and need no re-hiding. But
    // windows we adopted from the VISIBLE desktop (UWP brokered apps, browsers)
    // can reposition themselves back onto the screen, so we periodically
    // re-assert their off-screen position. We detect "visible-desktop" windows
    // by checking whether they are NOT on the hidden desktop.
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER, SWP_NOSIZE,
    };

    // Only re-assert occasionally to avoid per-frame SetWindowPos churn.
    if state.frame_counter % 60 != 0 {
        return;
    }

    // Collect HWNDs that are currently on the hidden desktop.
    let hidden: HashSet<isize> = snapshot_hidden_hwnds();

    for t in state.toplevels.iter() {
        if t.is_popup {
            continue;
        }
        let key = t.hwnd.0 as isize;
        // If it's on the hidden desktop it's already invisible — skip.
        if hidden.contains(&key) {
            continue;
        }
        // It's a visible-desktop window: if it has drifted back on-screen, push
        // it off-screen again (only when its top-left isn't already parked).
        let mut rc = RECT::default();
        unsafe {
            if GetWindowRect(t.hwnd, &mut rc).is_ok() && (rc.left > -20000 || rc.top > -20000) {
                let _ = SetWindowPos(
                    t.hwnd,
                    None,
                    -32000,
                    -32000,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
                );
            }
        }
    }
}


/// Detect a NEW top-level window spawned by the process tree of an
/// already-registered window, and register it as its OWN toplevel.
///
/// This is what makes "launch a game from inside Steam" (and Epic, GOG,
/// TLauncher, an installer that opens a second window, …) work. When the user
/// clicks Play in the Steam window we render, Steam spawns the GAME as a
/// SEPARATE child process. That game window is a brand-new top-level we never
/// launched directly, so it has NO `PendingLaunch` tracking it — the launch
/// detector (`poll_pending_launches`) already finished and dropped Steam's
/// single-instance launch. Without this pass the game window simply appears on
/// the (hidden) desktop and is never adopted: "games from Steam don't show up".
///
/// Strategy: gather the PID tree of every registered NON-popup toplevel, then
/// scan the hidden desktop for any real, titled top-level window that
///   * belongs to one of those process trees,
///   * is NOT already tracked, and
///   * is NOT itself a popup/menu.
/// Each such window is registered as a new toplevel (so it shows up as its own
/// window-item), and prepared like any hidden-desktop window.
/// Cheap gate for `poll_launcher_children`: return true only when at least one
/// tracked window belongs to a process whose EXECUTABLE looks like a game
/// LAUNCHER (Steam, Epic, GOG, Battle.net, EA, a Minecraft launcher, …). Only
/// launchers spawn a SEPARATE game window that the expensive launcher-children
/// scan needs to catch. When the only open windows are browsers, Discord,
/// Explorer, editors, etc. — which never spawn a standalone game window — we
/// skip the whole scan, which is what removes the periodic ~200 ms EnumWindows
/// spike for the common case. This check itself is cheap: it only reads each
/// tracked window's process image name (no window enumeration, no blocking
/// window messages).
pub fn should_scan_launcher_children(state: &WindowMod) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    for t in state.toplevels.iter().filter(|t| !t.is_popup) {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(t.hwnd, Some(&mut pid)) };
        if pid == 0 {
            continue;
        }
        // Read the process's executable path and check its file stem.
        let exe_stem: Option<String> = unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) if !handle.is_invalid() => {
                    let mut buf = [0u16; 260];
                    let mut len = buf.len() as u32;
                    let ok = QueryFullProcessImageNameW(
                        handle,
                        PROCESS_NAME_FORMAT(0),
                        windows::core::PWSTR(buf.as_mut_ptr()),
                        &mut len,
                    )
                    .is_ok();
                    let _ = CloseHandle(handle);
                    if ok && len > 0 {
                        let path = String::from_utf16_lossy(&buf[..len as usize]);
                        Some(hint_from_path(&path))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };
        if let Some(stem) = exe_stem {
            if is_launcher_executable(&stem) {
                return true;
            }
        }
    }
    false
}

/// True if `stem` (a lowercased exe file stem) is a known GAME LAUNCHER that
/// spawns games as separate child windows we must adopt.
fn is_launcher_executable(stem: &str) -> bool {
    matches!(
        stem,
        "steam"
            | "steamwebhelper"
            | "epicgameslauncher"
            | "galaxyclient"       // GOG Galaxy
            | "battle.net"
            | "battlenet"
            | "eadesktop"          // EA App
            | "origin"
            | "riotclientservices" // Riot
            | "vortex"
            | "tlauncher"
            | "minecraftlauncher"
            | "javaw"              // Minecraft / Java launchers
            | "java"
            | "playnite"
            | "uplay"
            | "upc"                // Ubisoft Connect
    )
}

pub fn poll_launcher_children(state: &mut WindowMod) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowLongW, GetWindowThreadProcessId,
        IsWindowVisible, GWL_STYLE, WS_CHILD, WS_VISIBLE,
    };



    // PIDs of every registered, non-popup toplevel (the launchers/apps we show).

    let mut tracked_pids: HashSet<u32> = HashSet::new();
    for t in state.toplevels.iter().filter(|t| !t.is_popup) {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(t.hwnd, Some(&mut pid)) };
        if pid != 0 {
            tracked_pids.insert(pid);
        }
    }
    if tracked_pids.is_empty() {
        return; // nothing registered yet → no launcher to watch
    }

    // Expand to the full descendant tree of each tracked PID (the game runs as a
    // child/grandchild of the launcher).
    let mut tree: HashSet<u32> = HashSet::new();
    for &root in &tracked_pids {
        for pid in collect_descendant_pids(root) {
            tree.insert(pid);
        }
    }

    // HWNDs we already track (toplevels AND popups) — never adopt twice.
    let known: HashSet<isize> = state.toplevels.iter().map(|t| t.hwnd.0 as isize).collect();

    // Find new, real, titled top-level windows on the hidden desktop owned by a
    // process in the tree but not yet tracked.
    let mut new_windows: Vec<HWND> = Vec::new();
    for_each_hidden_desktop_window(|hwnd| {
        let key = hwnd.0 as isize;
        if known.contains(&key) {
            return true;
        }
        let mut wpid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut wpid)) };
        // Must belong to a tracked launcher's process tree.
        if !tree.contains(&wpid) {
            return true;
        }
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
        if style & WS_CHILD.0 as i32 != 0 {
            return true;
        }
        // Skip INVISIBLE windows. A single app (modern Notepad, Chromium,
        // many .NET apps) owns several INVISIBLE helper/owner top-level windows
        // that nonetheless have a title and a non-tiny size — so they passed the
        // old title+size filter and got adopted as separate toplevels. They have
        // no rendered content, so PrintWindow returns an all-black bitmap: that
        // is the "unknown programs that show up as black squares" the user saw,
        // and the log confirmed one Notepad spawning 8+ `Notepad`-class
        // toplevels. Only genuinely VISIBLE windows are real, capturable app
        // windows, so require WS_VISIBLE (or IsWindowVisible) here.
        let visible = (style & WS_VISIBLE.0 as i32) != 0
            || unsafe { IsWindowVisible(hwnd).as_bool() };
        if !visible {
            return true;
        }
        // Skip popup/menu-class windows: those are handled by poll_popup_windows
        // and must render INSIDE their owner, not as a standalone window.
        let cls = class_name(hwnd);
        if cls == "#32768" {
            return true;
        }
        // Skip CHROMIUM/Electron windows. A single Chromium app (Opera, Discord,
        // Chrome) spawns MANY top-level `Chrome_WidgetWin_1` windows in its OWN
        // process tree — tab hosts, GPU-composition layers, helper/widget hosts.
        // The log showed Opera producing 8+ of them, each wrongly adopted here as
        // a separate "game" toplevel. That flooded the mod with a dozen windows,
        // each running its own PrintWindow capture thread and being memcpy'd
        // every frame — the real cause of the render lag. The app's MAIN window
        // is already registered by the launch path and its dropdowns by
        // poll_popup_windows, so this launcher-children pass must IGNORE Chromium
        // windows entirely; it exists only to catch a real GAME a launcher spawns.
        if cls == "Chrome_WidgetWin_1" || cls == "Chrome_WidgetWin_0" {
            return true;
        }
        // Require a real title and a non-tiny client area (skip helper windows).
        // win_title_len is non-blocking (no WM_GETTEXT) — a busy Chromium/Steam
        // window would otherwise stall this scan for ~1.5 s (the update() spike).
        let len = win_title_len(hwnd);

        if len <= 0 {
            return true;
        }
        let mut rc = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        if rc.right - rc.left >= 100 && rc.bottom - rc.top >= 100 {
            new_windows.push(hwnd);
        }
        true
    });



    // ALSO scan the VISIBLE desktop. Steam (and other launchers) frequently
    // start a game through a broker/reaper that does NOT inherit our hidden
    // desktop, so the game window appears on the user's VISIBLE desktop instead.
    // We find such windows by the same process tree, then PARK them off-screen
    // (so the user never sees them on their real desktop) and render them inside
    // Minecraft. This is the missing half of "games launched from Steam don't
    // show up": before we only looked on the hidden desktop and never found them.
    let mut new_visible: Vec<HWND> = Vec::new();
    {
        use windows::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetClientRect, GetWindowLongW, GetWindowTextLengthW,
            GetWindowThreadProcessId, IsWindowVisible, GWL_STYLE, WS_CHILD,
        };

        struct Search<'a> {
            tree: &'a HashSet<u32>,
            known: &'a HashSet<isize>,
            found: Vec<HWND>,
        }

        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let s = &mut *(lparam.0 as *mut Search);
            if s.known.contains(&(hwnd.0 as isize)) {
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
            // Skip Chromium/Electron helper windows here too (same reason as the
            // hidden-desktop branch): a browser spawns many Chrome_WidgetWin_1
            // top-levels and we must not adopt each as a separate "game".
            {
                use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
                let mut cbuf = [0u16; 64];
                let clen = GetClassNameW(hwnd, &mut cbuf);
                if clen > 0 {
                    let cls = String::from_utf16_lossy(&cbuf[..clen as usize]);
                    if cls == "Chrome_WidgetWin_1" || cls == "Chrome_WidgetWin_0" {
                        return TRUE;
                    }
                }
            }
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;

            // Games launched from Steam are the reason we relaxed the title
            // requirement here. A DirectX/Vulkan game frequently creates its
            // main window with NO title text (and sometimes borderless
            // fullscreen), so the old `GetWindowTextLengthW <= 0` reject made
            // these games never get adopted ("Steam games don't show up").
            //
            // We replace that with a size-based heuristic for UNTITLED windows:
            //   * a titled window is accepted at the usual >= 100x100, but
            //   * an UNTITLED window must be reasonably LARGE (>= 200x200) to be
            //     treated as a game window rather than an invisible 1x1 helper /
            //     splash / message window the same process owns.
            // Both must be non-child, visible, and in the launcher's process
            // tree (checked above), so this stays specific to real game windows.
            let len = GetWindowTextLengthW(hwnd);
            let big_enough = if len > 0 {
                w >= 100 && h >= 100
            } else {
                w >= 200 && h >= 200
            };
            if big_enough {
                s.found.push(hwnd);
            }
            TRUE

        }

        let mut search = Search { tree: &tree, known: &known, found: Vec::new() };
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(&mut search as *mut _ as isize));
        }
        new_visible = search.found;
    }

    for hwnd in new_windows {
        eprintln!(
            "[windowmod] poll_launcher_children: adopting new launcher-child window HWND {:?} class='{}' (hidden desktop)",
            hwnd, class_name(hwnd),
        );
        if register_external_hwnd(state, hwnd, "game".to_string()).is_some() {
            // A launcher child is very likely a GAME. Force it out of any
            // exclusive/borderless-fullscreen mode into a normal windowed frame
            // so DWM composites it and WGC can capture it (see make_game_window).
            make_game_window(hwnd);
        }
    }

    for hwnd in new_visible {
        eprintln!(
            "[windowmod] poll_launcher_children: adopting new launcher-child window HWND {:?} class='{}' (VISIBLE desktop — parking off-screen)",
            hwnd, class_name(hwnd),
        );
        if register_external_hwnd(state, hwnd, "game".to_string()).is_some() {
            // De-fullscreen the game first, THEN park it off-screen. A game left
            // in borderless/exclusive fullscreen keeps trying to own the whole
            // (real) screen and won't composite for capture.
            make_game_window(hwnd);
            park_offscreen(hwnd);
        }
    }
}

/// Prepare a launched GAME window so it can be captured inside Minecraft.
///
/// DirectX/Vulkan games are the hard case: many launch in EXCLUSIVE or
/// BORDERLESS FULLSCREEN, where the app owns the whole screen's swap-chain. On
/// our hidden (non-interactive) desktop that swap-chain has no real output to
/// present to, so the game either shows a black frame, fails to create its
/// device, or "does not open". Even a borderless-fullscreen window (WS_POPUP
/// covering the monitor) does not composite the way WGC needs.
///
/// The fix is to force the game into a NORMAL, overlapped WINDOW: give it a
/// title bar / border style (WS_OVERLAPPEDWINDOW) and clear the WS_POPUP /
/// maximize / minimize bits. A windowed game renders through a normal
/// DWM-composited swap-chain, which is exactly what WGC captures. We then show
/// it normally so it keeps a real client size. This is the same reason desktop
/// games "run fine in windowed mode" when a capture/streaming tool is involved.
fn make_game_window(hwnd: HWND) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos, ShowWindow, GWL_STYLE,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SW_SHOWNORMAL, WS_MINIMIZE, WS_MAXIMIZE, WS_OVERLAPPEDWINDOW, WS_POPUP,
        };

        eprintln!("[windowmod] Preparing GAME window HWND {:?} (forcing windowed)", hwnd);

        let style = GetWindowLongW(hwnd, GWL_STYLE);
        // Drop the bits that keep a game fullscreen/borderless and add a normal
        // overlapped window frame so DWM composites it for capture.
        let mut new_style = style;
        new_style &= !(WS_POPUP.0 as i32);
        new_style &= !(WS_MINIMIZE.0 as i32 | WS_MAXIMIZE.0 as i32);
        new_style |= WS_OVERLAPPEDWINDOW.0 as i32;
        if new_style != style {
            let _ = SetWindowLongW(hwnd, GWL_STYLE, new_style);
            // SWP_FRAMECHANGED makes the style change take effect immediately.
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        // Show it normally on the hidden desktop so it keeps a real size and
        // renders through the compositor (invisible to the user regardless,
        // because the desktop itself is not the active one).
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
    }
}



/// Scan the hidden desktop for newly-appeared POPUP windows (dropdown menus,
/// context menus, combo lists, flyouts) and register them as regular toplevels
/// so they are captured and rendered in Minecraft.
///
/// Why this exists: dropdown menus (File/View ... → Save As, Font, ...) are NOT
/// drawn inside the owning window. They are SEPARATE top-level popup windows
/// (classic class `#32768`, or owned WS_POPUP windows for XAML/WinUI flyouts)
/// that the app creates on demand. Because they belong to our hidden desktop
/// but are brand-new top-levels, the normal launch-detection path never sees
/// them. We poll for them here every frame.
///
/// We register them via `register_popup_hwnd`, which (unlike the launch path)
/// accepts small, title-less windows — menus are exactly that. They become
/// ordinary toplevels, so capture, input routing and Java rendering all work
/// with zero extra plumbing. When the menu closes the window is destroyed and
/// `retain_toplevels` drops it on the next frame.
pub fn poll_popup_windows(state: &mut WindowMod) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindow, GetWindowLongW, GetWindowThreadProcessId, GW_OWNER, GWL_STYLE,
        WS_CHILD, WS_VISIBLE,
    };

    // 1) Refresh the offset of popups we already track so they stay anchored
    //    inside the owner window if it moves/resizes, and drop dead ones.
    {
        // (toplevel_ptr, popup_hwnd, owner_hwnd) for every tracked popup.
        let popups: Vec<(i64, HWND, HWND)> = state
            .toplevels
            .iter()
            .filter(|t| t.is_popup)
            .map(|t| (super::state::ptr_of_ref(&**t), t.hwnd, t.owner_hwnd))
            .collect();
        for (tl_ptr, popup_hwnd, owner_hwnd) in popups {
            if !super::capture::hwnd_alive(popup_hwnd) || !super::capture::hwnd_alive(owner_hwnd) {
                continue;
            }
            let (xoff, yoff) = super::capture::refresh_popup_offset(popup_hwnd, owner_hwnd);
            for s in state.surfaces.iter_mut() {
                if s.toplevel_ptr == tl_ptr {
                    if s.xoff != xoff || s.yoff != yoff {
                        s.xoff = xoff;
                        s.yoff = yoff;
                        s.buffer_dirty = true;
                    }
                }
            }
        }
    }

    // Map of owning toplevel: HWND -> (toplevel_ptr, root surface_ptr).
    // Only NON-popup toplevels can own a popup. We anchor popups inside their
    // owner's window, so we need both the owner HWND (for offset) and its root
    // surface ptr (to parent the popup's surface).
    let owners: Vec<(HWND, i64)> = state
        .toplevels
        .iter()
        .filter(|t| !t.is_popup)
        .filter_map(|t| {
            let tl_ptr = super::state::ptr_of_ref(&**t);
            state.surface_for_toplevel(tl_ptr).map(|s_ptr| (t.hwnd, s_ptr))
        })
        .collect();
    if owners.is_empty() {
        return; // no real windows yet → nothing can own a popup
    }

    // PIDs of our owning toplevels — a popup typically shares the owner's PID.
    let mut owned_pids: HashSet<u32> = HashSet::new();
    for (hwnd, _) in &owners {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(*hwnd, Some(&mut pid)) };
        if pid != 0 {
            owned_pids.insert(pid);
        }
    }

    let known: HashSet<isize> = state.toplevels.iter().map(|t| t.hwnd.0 as isize).collect();

    // Collect new popup windows together with the owner toplevel they belong to.
    let mut new_popups: Vec<(HWND, HWND, i64)> = Vec::new();
    for_each_hidden_desktop_window(|hwnd| {
        let key = hwnd.0 as isize;
        if known.contains(&key) {
            return true;
        }
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
        // Must be visible and not a child window.
        if style & WS_VISIBLE.0 as i32 == 0 {
            return true;
        }
        if style & WS_CHILD.0 as i32 != 0 {
            return true;
        }
        // Only consider popup-class windows: classic menus (`#32768`) or
        // WS_POPUP windows (XAML/WinUI flyouts, combo dropdowns).
        let cls = class_name(hwnd);
        let is_menu_class = cls == "#32768";
        let is_popup_style = (style as u32 & 0x8000_0000) != 0; // WS_POPUP
        if !is_menu_class && !is_popup_style {
            return true;
        }

        // Determine the OWNER toplevel. This is the crucial guard that prevents
        // mistaking an application's MAIN window (which may itself be WS_POPUP,
        // e.g. many WinUI/Electron apps) for a dropdown: a real menu/flyout has
        // an owner window, an app's main window does not (its GW_OWNER is null
        // and it is not owned by any toplevel we track).
        let owner_hwnd = unsafe { GetWindow(hwnd, GW_OWNER) }.unwrap_or(HWND(std::ptr::null_mut()));

        // Resolve which tracked owner toplevel this popup belongs to:
        //   a) its GW_OWNER is one of our toplevels, OR
        //   b) it is a classic `#32768` menu sharing a tracked toplevel's PID
        //      (classic menus often report a null owner).
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };

        let owner_match = owners.iter().find(|(ohwnd, _)| {
            if !owner_hwnd.0.is_null() && *ohwnd == owner_hwnd {
                return true;
            }
            if is_menu_class {
                let mut opid = 0u32;
                unsafe { GetWindowThreadProcessId(*ohwnd, Some(&mut opid)) };
                return opid == pid && owned_pids.contains(&pid);
            }
            false
        });

        let Some((owner, owner_surface_ptr)) = owner_match.copied() else {
            return true; // not a popup of any window we own → ignore
        };

        // Require a real, non-tiny client area.
        let mut rc = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        if rc.right - rc.left >= 4 && rc.bottom - rc.top >= 4 {
            new_popups.push((hwnd, owner, owner_surface_ptr));
        }
        true
    });

    for (hwnd, owner, owner_surface_ptr) in new_popups {
        eprintln!(
            "[windowmod] poll_popup_windows: registering popup HWND {:?} class='{}' owner={:?}",
            hwnd, class_name(hwnd), owner,
        );
        if super::capture::register_popup_hwnd(state, hwnd, owner, owner_surface_ptr).is_some() {
            make_compositor_window(hwnd);
        }
    }
}


fn class_name(hwnd: HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

// InternalGetWindowText reads a window's title from the kernel-cached copy
// WITHOUT sending a synchronous WM_GETTEXT to the window's own thread. The
// documented GetWindowTextW/GetWindowTextLengthW send WM_GETTEXT, which BLOCKS
// until the target app pumps it — for a busy app (Opera playing a video, Steam,
// a loading game) that froze Minecraft's render thread for ~1.5 SECONDS each
// time the window-scan read a title (the log showed `native update() took
// 1565 ms`). Every title read in the hot window-scan paths below goes through
// these non-blocking helpers instead, so a busy or hung app can never stall us.
#[link(name = "user32")]
extern "system" {
    fn InternalGetWindowText(hwnd: HWND, psz: *mut u16, cch: i32) -> i32;
}

/// Non-blocking window title (never sends WM_GETTEXT). Empty if untitled.
fn win_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let read = unsafe { InternalGetWindowText(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if read <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..read as usize])
    }
}

/// Non-blocking title length (never sends WM_GETTEXT). 0 if untitled.
fn win_title_len(hwnd: HWND) -> i32 {
    let mut buf = [0u16; 256];
    unsafe { InternalGetWindowText(hwnd, buf.as_mut_ptr(), buf.len() as i32) }
}



/// Find the main top-level window on the hidden desktop whose title contains
/// `primary` (or `alt`). Used as a fallback when PID matching fails (e.g. the
/// launcher process forks into another PID).
fn find_hidden_window_by_hint(primary: &str, alt: &Option<String>) -> Option<HWND> {
    find_hidden_by_single_hint(primary)
        .or_else(|| alt.as_deref().and_then(find_hidden_by_single_hint))
}

fn find_hidden_by_single_hint(hint: &str) -> Option<HWND> {
    if hint.is_empty() {
        return None;
    }
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowLongW, GWL_STYLE, WS_CHILD,
    };

    let needle = hint.to_lowercase();
    let mut found: Option<HWND> = None;
    for_each_hidden_desktop_window(|hwnd| {
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
        if style & WS_CHILD.0 as i32 != 0 {
            return true;
        }
        // Non-blocking title read (win_title) — a busy app would otherwise stall
        // this whole-desktop scan on WM_GETTEXT for ~1.5 s.
        let title = win_title(hwnd).to_lowercase();
        if title.is_empty() {
            return true;
        }
        if !title.contains(&needle) {
            return true;
        }

        // Require a real, non-tiny client area (skip 1x1 helper windows).
        let mut rc = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut rc); }
        if rc.right - rc.left >= 10 && rc.bottom - rc.top >= 10 {
            found = Some(hwnd);
            return false; // stop
        }
        true
    });
    found
}

/// Terminate every process the mod launched from inside Minecraft, plus their
/// descendant process trees. Called from `shutdown` when Minecraft exits so
/// apps the user opened through the window mod do not linger after the game is
/// gone. Processes that were already running before the mod launched them are
/// never in `launched_pids`, so they are left untouched.
pub fn kill_launched_processes(state: &WindowMod) {
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_TERMINATE,
    };

    if state.launched_pids.is_empty() {
        return;
    }

    // Expand each launched root PID into its full descendant tree (browsers,
    // launchers and Electron apps fork many child processes), so we terminate
    // the whole app rather than leaving orphaned children behind.
    let mut all: HashSet<u32> = HashSet::new();
    for &root in &state.launched_pids {
        for pid in collect_descendant_pids(root) {
            all.insert(pid);
        }
    }

    eprintln!(
        "[windowmod] kill_launched_processes: terminating {} process(es) launched by the mod",
        all.len(),
    );

    for pid in all {
        if pid == 0 {
            continue;
        }
        unsafe {
            match OpenProcess(PROCESS_TERMINATE, false, pid) {
                Ok(handle) if !handle.is_invalid() => {
                    let _ = TerminateProcess(handle, 0);
                    let _ = CloseHandle(handle);
                }
                _ => {
                    // Process already gone (or no permission) — nothing to do.
                }
            }
        }
    }
}

/// Force-terminate the process (and its whole descendant tree) that owns the
/// window backing `toplevel_ptr`. Used by the "Force Quit" button in the window
/// manager to kill an app whose window is hung / won't close normally.
///
/// We resolve the window's PID, expand it into the full descendant tree (a
/// browser / Electron app spawns many child processes), and `TerminateProcess`
/// each one — the same mechanism `kill_launched_processes` uses on shutdown.
/// This works regardless of whether the mod launched the app or the user did.
pub fn kill_toplevel(state: &mut WindowMod, toplevel_ptr: i64) -> bool {
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    // Only proceed if the pointer still corresponds to a LIVE toplevel owned by
    // state.toplevels (guards against a dangling pointer from a freed window).
    if !state
        .toplevels
        .iter()
        .any(|t| super::state::ptr_of_ref(&**t) == toplevel_ptr)
    {
        eprintln!("[windowmod] kill_toplevel: ptr={} not a live toplevel", toplevel_ptr);
        return false;
    }
    let Some(t) = super::state::ptr_to_ref::<super::state::WinToplevel>(toplevel_ptr) else {
        eprintln!("[windowmod] kill_toplevel: ptr={} resolves to no toplevel", toplevel_ptr);
        return false;
    };

    let hwnd = t.hwnd;

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        eprintln!("[windowmod] kill_toplevel: no PID for HWND {:?}", hwnd);
        return false;
    }

    // Expand into the full descendant process tree so child renderer/helper
    // processes (Chromium, Electron, launchers) are terminated too.
    let mut all: HashSet<u32> = HashSet::new();
    for p in collect_descendant_pids(pid) {
        all.insert(p);
    }
    all.insert(pid);

    eprintln!(
        "[windowmod] kill_toplevel: force-terminating {} process(es) for HWND {:?} (root pid {})",
        all.len(), hwnd, pid,
    );

    let mut any = false;
    for p in all {
        if p == 0 {
            continue;
        }
        unsafe {
            match OpenProcess(PROCESS_TERMINATE, false, p) {
                Ok(handle) if !handle.is_invalid() => {
                    let _ = TerminateProcess(handle, 0);
                    let _ = CloseHandle(handle);
                    any = true;
                }
                _ => {}
            }
        }
    }

    // Mark the toplevel unmapped so retain_toplevels prunes it promptly once the
    // window disappears; the capture thread exits on the next tick.
    if let Some(tm) = super::state::ptr_to_mut::<super::state::WinToplevel>(toplevel_ptr) {
        tm.mapped = false;
    }
    let _ = state;
    any
}

fn find_by_hint(primary: &str, alt: &Option<String>) -> Option<HWND> {
    find_new_window_by_hint(primary)
        .or_else(|| alt.as_deref().and_then(find_new_window_by_hint))
}



fn find_new_window_by_hint(hint: &str) -> Option<HWND> {
    if hint.is_empty() {
        return None;
    }
    use windows::Win32::Foundation::{BOOL, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

    struct Search {
        hint: String,
        found: Option<HWND>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);

        use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        if ex_style & WS_EX_LAYERED.0 as i32 != 0 {
            return TRUE;
        }
        if ex_style & WS_EX_TOOLWINDOW.0 as i32 != 0 {
            return TRUE;
        }

        // Non-blocking title read (win_title) — sending WM_GETTEXT to every
        // top-level window here is what stalled the render thread ~1.5 s on a
        // busy app and caused the periodic freeze.
        let title = win_title(hwnd).to_lowercase();
        if title.is_empty() {
            return TRUE;
        }
        if title.contains(&search.hint) {
            search.found = Some(hwnd);
            return BOOL(0);
        }
        TRUE
    }


    let mut search = Search {
        hint: hint.to_lowercase(),
        found: None,
    };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}
