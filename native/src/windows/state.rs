use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use windows::Win32::Foundation::HWND;


use super::apps;

/// Native handle exposed to Java as a jlong pointer.
pub struct WinToplevel {
    pub hwnd: HWND,
    pub title: String,
    pub app_id: String,
    pub width: i32,
    pub height: i32,
    pub geom_x: i32,
    pub geom_y: i32,
    pub geom_w: i32,
    pub geom_h: i32,
    /// Latest captured pixels (BGRA, top-down, stride = width*4) shared straight
    /// from the capture thread via an `Arc`. Storing the Arc (instead of a
    /// separate Vec we memcpy into every frame) lets `refresh_windows` publish a
    /// new frame with just an atomic refcount bump — no ~8 MB copy on the render
    /// thread — which is what keeps native update() fast and the windows smooth.
    pub buffer: Arc<Vec<u8>>,
    pub mapped: bool,
    pub maximize: bool,

    pub fullscreen: bool,
    pub requests: ToplevelRequests,
    pub zero_capture_count: u32,
    pub prev_first16: [u8; 16],
    pub static_frame_count: u32,
    /// Version of the last shared frame copied from the background capture
    /// thread. Used to avoid re-uploading an unchanged frame.
    pub last_frame_version: u64,
    /// True if this toplevel is actually a popup (dropdown/context menu, combo
    /// list, flyout). Popups are captured exactly like toplevels, but they are
    /// NOT returned by the JNI `toplevels()` call — instead their surface is
    /// linked as a CHILD of the owning toplevel's surface tree, so they render
    /// INSIDE the owning window (like a real dropdown) rather than as a
    /// separate floating Minecraft window.
    pub is_popup: bool,
    /// For a popup: the HWND of the owning toplevel window. Used to compute the
    /// popup's offset relative to the owner's client area.
    pub owner_hwnd: HWND,
}



#[derive(Default)]
pub struct ToplevelRequests {
    pub minimize: bool,
    pub maximize: bool,
    pub unmaximize: bool,
    pub fullscreen: bool,
    pub unfullscreen: bool,
}

pub struct WinSurface {
    pub toplevel_ptr: i64,
    pub parent_ptr: i64,
    pub xoff: i32,
    pub yoff: i32,
    pub damage: Vec<[i32; 4]>,
    pub buffer_dirty: bool,
}

pub struct PendingLaunch {
    pub pid: u32,
    pub app_id: String,
    pub attempts: u32,
    pub hint: String,
    pub alt_hint: Option<String>,
    /// Snapshot of HWND values (as isize) taken before/around launch.
    pub snapshot: HashSet<isize>,
    /// True if the app was relaunched without SW_HIDE (fallback when SW_HIDE
    /// prevented window creation, e.g. Java Control Panel).
    pub relaunched: bool,
    /// Set to the HWND isize once the window is found and registered.
    /// After registration the pending launch stays alive for 30 frames
    /// to catch additional windows, then is removed automatically.
    pub hwnd_found: Option<isize>,
    pub hwnd_found_attempt: u32,
    pub rejected_hwnds: HashSet<isize>,
    /// The native WinToplevel pointer we created/reused for this launch's
    /// window. When the app swaps its window (a launcher opening the launched
    /// app, a splash being replaced by the main window, …) we REUSE this same
    /// toplevel pointer for the new HWND instead of creating a new toplevel, so
    /// any window-item the player is holding (which stores this pointer) keeps
    /// working and automatically shows the new window.
    pub toplevel_ptr: Option<i64>,
    /// The PID that originally launched. We track this process AND all of its
    /// descendant processes, so a launcher (TLauncher, Steam, an installer) that
    /// spawns the real app as a CHILD process still has that child's window
    /// adopted into the same toplevel/item.
    pub root_pid: u32,
}


pub struct WindowMod {
    pub toplevels: Vec<Box<WinToplevel>>,
    pub surfaces: Vec<Box<WinSurface>>,
    pub pending_launches: Vec<PendingLaunch>,
    pub output_size: (i32, i32),
    pub output_bounds: (i32, i32),
    pub focus_toplevel_ptr: i64,
    pub pointer_focus_ptr: i64,
    pub pointer_x: f64,
    pub pointer_y: f64,
    pub pointer_locked: bool,
    /// True while the left mouse button is held down (between LBUTTONDOWN and
    /// LBUTTONUP). Used so WM_MOUSEMOVE carries the MK_LBUTTON flag during a
    /// drag, which is what makes text-selection (click-and-drag) work in edit
    /// controls. Also used to suppress the UIA Invoke path on drag, since a
    /// drag is a selection gesture, not a button activation.
    pub left_button_down: bool,
    pub keyboard_active: bool,

    pub move_serial: Option<u32>,

    pub resize_serial: Option<(u32, i32)>,
    pub serial_counter: u32,
    pub desktop_apps: Vec<apps::DesktopApp>,
    pub preferred_terminal: String,
    /// Root PIDs of every process WE launched from inside Minecraft (via the app
    /// launcher). On shutdown we terminate these and their descendant trees so
    /// apps the user opened through the mod don't linger after Minecraft exits.
    /// Processes that were already running BEFORE the mod launched them are
    /// never in this list, so they are left untouched.
    pub launched_pids: Vec<u32>,

    /// Increments every `update()` call. Used to throttle the expensive
    /// `PrintWindow` capture so it does not run on every single frame.
    pub frame_counter: u64,
}

/// Capture at most once every N `update()` calls. PrintWindow on a large
/// window costs 10-19 ms; running it inline on the render thread stalls the
/// frame. This is a stop-gap throttle — the real fix is moving capture off the
/// render thread. Interval 3 keeps the window fairly live (~20 captures/sec).
const CAPTURE_INTERVAL: u64 = 3;




impl WindowMod {
    pub fn new() -> Self {
        Self {
            toplevels: Vec::new(),
            surfaces: Vec::new(),
            pending_launches: Vec::new(),
            output_size: (1920, 1080),
            output_bounds: (1920, 1080),
            focus_toplevel_ptr: 0,
            pointer_focus_ptr: 0,
            pointer_x: 0.0,
            pointer_y: 0.0,
            pointer_locked: false,
            left_button_down: false,
            keyboard_active: false,
            move_serial: None,

            resize_serial: None,
            serial_counter: 1,
            desktop_apps: apps::load_start_menu_apps(),
            preferred_terminal: String::new(),
            launched_pids: Vec::new(),
            frame_counter: 0,

        }
    }


    pub fn next_serial(&mut self) -> u32 {
        let s = self.serial_counter;
        self.serial_counter = self.serial_counter.wrapping_add(1);
        s
    }

    pub fn insert_toplevel(&mut self, toplevel: WinToplevel) -> i64 {
        self.toplevels.push(Box::new(toplevel));
        ptr_of(self.toplevels.last_mut().unwrap().as_mut())
    }

    pub fn insert_surface(&mut self, surface: WinSurface) -> i64 {
        self.surfaces.push(Box::new(surface));
        ptr_of(self.surfaces.last_mut().unwrap().as_mut())
    }

    pub fn surface_for_toplevel(&self, toplevel_ptr: i64) -> Option<i64> {
        self.surfaces
            .iter()
            .find(|s| s.toplevel_ptr == toplevel_ptr && s.parent_ptr == 0)
            .map(|s| ptr_of_ref(&**s))
    }

    pub fn update(&mut self) {
        self.frame_counter = self.frame_counter.wrapping_add(1);
        let _ = CAPTURE_INTERVAL;

        // Capture runs on a background thread; refresh_windows only copies the
        // latest finished frame — cheap, safe to call every frame.
        super::capture::refresh_windows(self);
        retain_toplevels(self);

        // The three window-scanning polls below each enumerate every top-level
        // window on the hidden (and sometimes visible) desktop and query class
        // names / client rects. On a busy system that costs several ms, and when
        // two of them landed on the SAME frame the render thread stalled for
        // 100+ ms (the "native update() took 118 ms" spikes that make apps lag
        // and hang). The fix is twofold:
        //   1) NEVER run more than ONE heavy scan on any given frame — they are
        //      scheduled on DIFFERENT residues of the frame counter so their
        //      cost is spread out instead of stacking.
        //   2) Run them LESS often. Launch/child/hidden-reassert detection does
        //      not need to happen 2x/sec; menus are the only latency-sensitive
        //      case and get their own faster (but still staggered) cadence.

        // WINDOW DISCOVERY IS NOW OFF THE RENDER THREAD. The launcher-children,
        // popup and off-screen-rehide passes used to run here and call blocking
        // Win32 enumeration on possibly-busy foreign windows, stalling the
        // render thread (the 30-236 ms spikes / browser freezes). All of that
        // heavy work now runs on the dedicated `scanner` background thread; the
        // render thread only publishes a cheap snapshot and applies the results.

        // Publish tracked-window snapshot for the scanner ~6x/sec (a few PID
        // reads; no window enumeration).
        if self.frame_counter % 10 == 0 {
            super::process::publish_scan_input(self);
        }

        // Apply the scanner's found windows every frame — cheap (register-only)
        // and gives newly-opened menus/games minimal latency.
        super::process::apply_scan_output(self);

        // poll_pending_launches is the ONE scan kept on the render thread: it
        // owns the render-thread pending-launch list and early-outs to a single
        // IsWindow check once a launch resolves. Run it only while a launch is
        // pending, every 20 frames.
        if !self.pending_launches.is_empty() && self.frame_counter % 20 == 0 {
            super::process::poll_pending_launches(self);
        }
    }










}

pub fn ptr_of<T>(value: &mut T) -> i64 {
    (value as *mut T) as i64
}

pub fn ptr_of_ref<T>(value: &T) -> i64 {
    (value as *const T) as i64
}

pub fn ptr_to_ref<T>(ptr: i64) -> Option<&'static T> {
    if ptr == 0 {
        return None;
    }
    Some(unsafe { &*(ptr as usize as *const T) })
}

pub fn ptr_to_mut<T>(ptr: i64) -> Option<&'static mut T> {
    if ptr == 0 {
        return None;
    }
    Some(unsafe { &mut *(ptr as usize as *mut T) })
}

pub fn retain_toplevels(state: &mut WindowMod) {
    let toplevels_before = state.toplevels.len();
    let surfaces_before = state.surfaces.len();

    // Drop toplevels whose window has unmapped or whose HWND is gone. (We do NOT
    // "pin" toplevels referenced by a pending launch: keeping a toplevel alive
    // after its real window died left it capturing nothing, so the surface
    // rendered solid BLACK. A genuine splash→main swap is instead handled by
    // re-registering the new window in `poll_pending_launches`.)
    state.toplevels.retain(|t| t.mapped && super::capture::hwnd_alive(t.hwnd));



    let alive: HashMap<i64, ()> = state
        .toplevels
        .iter()
        .map(|t| ptr_of_ref(&**t))
        .map(|p| (p, ()))
        .collect();
    state
        .surfaces
        .retain(|s| alive.contains_key(&s.toplevel_ptr));
    let removed_t = toplevels_before - state.toplevels.len();
    let removed_s = surfaces_before - state.surfaces.len();
    if removed_t > 0 || removed_s > 0 {
        eprintln!(
            "[windowmod] retain_toplevels: removed {} toplevels, {} surfaces (remaining: {} t, {} s)",
            removed_t, removed_s, state.toplevels.len(), state.surfaces.len(),
        );
    }
}
