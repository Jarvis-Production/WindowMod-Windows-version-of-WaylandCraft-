use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};





use windows::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    GetDIBits, HDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowTextLengthW, GetWindowTextW, GetWindowLongW, GWL_STYLE,
    IsWindow, WS_CHILD,
};

use super::state::{ptr_of, WinSurface, WinToplevel, WindowMod};

// PrintWindow is not in windows crate v0.58 feature set — call via raw FFI
#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: HWND, hdc_dest: HDC, flags: u32) -> BOOL;
}
const PW_CLIENTONLY: u32 = 0x00000001;
const PW_RENDERFULLCONTENT: u32 = 0x00000002;


// ===========================================================================
// Off-thread capture
//
// PrintWindow + GetDIBits cost 10-19 ms on a large window. Running that on
// Minecraft's render thread stalls every frame. Instead a dedicated background
// thread continuously captures every registered window into a shared buffer,
// and the render thread merely copies the latest finished frame (a cheap
// memcpy). This decouples window capture rate from the game's framerate.
// ===========================================================================

/// A finished capture for one HWND, produced by the background thread.
struct SharedFrame {
    width: i32,
    height: i32,
    /// BGRA, top-down, tightly packed (stride = width * 4). Stored behind an Arc
    /// so the render thread can grab a reference under the lock in O(1) (just an
    /// atomic refcount bump) instead of memcpy'ing ~8 MB while holding the mutex
    /// — that memcpy-under-lock serialized against the capture thread and spiked
    /// frame time into the hundreds of ms once more than one window existed.
    data: Arc<Vec<u8>>,
    /// Bumped each time `data` is refreshed, so the render thread can tell
    /// whether there is a new frame to upload.
    version: u64,
    /// Cheap sampled checksum of the last published pixels. Used to SKIP
    /// re-publishing (and bumping `version`) when a freshly captured frame is
    /// pixel-identical to the previous one. Without this, a STATIC window
    /// (Explorer sitting still, a paused game, an open menu) bumped `version`
    /// 60×/sec, forcing the render thread to re-upload its (large) texture every
    /// frame for every window — pure wasted CPU/GPU that showed up as lag.
    checksum: u64,
}

/// Compute a cheap FNV-1a-style checksum over a SAMPLE of the buffer (every
/// `STRIDE`-th byte), so detecting "did this frame change?" costs a tiny
/// fraction of a full ~8 MB compare while still reliably catching real changes.
fn sampled_checksum(buf: &[u8]) -> u64 {
    // Sample ~one byte per 64 — enough to detect any visible change, cheap
    // enough to run every capture for every window.
    const STRIDE: usize = 64;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < buf.len() {
        hash ^= buf[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        i += STRIDE;
    }
    // Fold in the length so a size change always changes the checksum.
    hash ^= buf.len() as u64;
    hash
}



/// Latest finished frame per HWND, published by that window's own capture
/// thread and consumed (copied) by the render thread.
static CAPTURE_FRAMES: OnceLock<Mutex<HashMap<isize, SharedFrame>>> = OnceLock::new();

/// The set of HWNDs that currently have a live capture thread. Used by
/// `ensure_per_window_threads` to spawn a thread for newly-appeared windows and
/// to tell existing threads to exit when their window is no longer a target.
static CAPTURE_LIVE: OnceLock<Mutex<HashSet<isize>>> = OnceLock::new();

fn frames() -> &'static Mutex<HashMap<isize, SharedFrame>> {
    CAPTURE_FRAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn live_threads() -> &'static Mutex<HashSet<isize>> {
    CAPTURE_LIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// HWND (as isize) of the window the player is currently LOOKING AT in the
/// window-manager screen. Only ONE window is rendered on screen at a time (the
/// focused toplevel); every other window is off-screen and its live pixels are
/// invisible to the player right now.
///
/// The PrintWindow fallback is expensive (a full GPU/GDI blit, 10-19 ms on a
/// large window). Running it at 60 Hz for EVERY open window burned CPU on
/// content nobody is viewing — with a dozen windows that stacked into the lag
/// the user reported. Capture threads read this to pick their cadence: the
/// focused window captures fast (smooth), all others capture slowly (just often
/// enough that switching to them shows fresh content within a fraction of a
/// second). Updated by the render thread each frame.
static FOCUSED_CAPTURE_HWND: AtomicIsize = AtomicIsize::new(0);

/// Set which window is currently displayed/focused so its capture thread runs
/// at full rate and the rest throttle down. Called from the render thread.
pub fn set_focused_capture_hwnd(hwnd: isize) {
    FOCUSED_CAPTURE_HWND.store(hwnd, Ordering::Relaxed);
}


/// Ensure that EVERY window in `targets` has its OWN dedicated capture thread,
/// and that windows no longer in `targets` have their thread asked to stop.
///
/// WHY ONE THREAD PER WINDOW (the freeze fix):
/// The previous design ran a SINGLE background thread that captured every window
/// SEQUENTIALLY in one loop. `PrintWindow`/`GetDC` on a busy GPU-composited
/// window (Opera/Discord/Electron after a while, a window whose app stops
/// pumping messages, DWM hiccups) can BLOCK for hundreds of ms to seconds. With
/// one shared loop, that single stalled `PrintWindow` froze the capture of ALL
/// windows at once — so every window "hung" and stopped updating even though the
/// apps were alive. That is exactly the "Opera works then freezes, and so does
/// everything" symptom.
///
/// Giving each window its own thread ISOLATES a slow/stuck `PrintWindow`: only
/// that one window's frames pause; every other window keeps updating in real
/// time. A thread exits on its own once its HWND leaves the target set or dies.
fn ensure_per_window_threads(targets: &[isize]) {
    let mut live = match live_threads().lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    // Stop threads whose window is no longer a target: just drop it from the
    // live set. The thread checks this set each tick and exits when absent.
    live.retain(|h| targets.contains(h));

    // Spawn a thread for any target that does not yet have one.
    for &hwnd_isize in targets {
        if live.contains(&hwnd_isize) {
            continue;
        }
        live.insert(hwnd_isize);
        std::thread::spawn(move || capture_one_window(hwnd_isize));
    }
}

/// Dedicated capture loop for a SINGLE window. Runs until the window dies or is
/// removed from the live set. Because it owns only one HWND, a slow or hung
/// `PrintWindow` here can never stall any other window's capture.
fn capture_one_window(hwnd_isize: isize) {
    let hwnd = HWND(hwnd_isize as *mut _);

    // CRITICAL: bind THIS capture thread to the hidden desktop BEFORE creating
    // any WGC/COM objects. Windows Graphics Capture's `CreateForWindow` fails
    // with 0x80070057 (E_INVALIDARG) when the target window is on a DIFFERENT
    // desktop than the calling thread. Our app windows live on the hidden
    // `WindowModDesktop` while this thread would otherwise be on the process's
    // default desktop — which is exactly why the log showed WGC failing for
    // EVERY window and everything falling back to the slow, black-frame
    // PrintWindow path (the lag/hang). SetThreadDesktop moves us onto the hidden
    // desktop so WGC can capture these windows.
    let bound = super::process::bind_thread_to_hidden_desktop();
    eprintln!(
        "[windowmod] capture_one_window: bind_thread_to_hidden_desktop -> {} for HWND {:?}",
        bound, hwnd,
    );

    // Per-thread scratch buffer, grown as needed and reused across frames.
    let mut scratch: Vec<u8> = Vec::new();

    // Capture cadence. We want the window content to feel as live as the real
    // desktop (matching a 100+ FPS game), but we must not spin the CPU or hammer
    // the shared `frames()` mutex the render thread also locks.
    //
    // Two cadences:
    //   * WGC path — `TryGetNextFrame` is CHEAP and returns None when the window
    //     produced no new frame, so we can poll it FAST (~4 ms ≈ 240 Hz). We only
    //     ever lock `frames()` / allocate when a genuinely NEW, CHANGED frame
    //     arrives, so a static window polled at 240 Hz costs almost nothing (a
    //     null TryGetNextFrame + a short sleep). This is what makes animating
    //     content (video, scrolling, games) look smooth at high game FPS instead
    //     of capped at 60.
    //   * PrintWindow fallback — each grab is a heavy GDI blit (10-19 ms), so we
    //     keep it at ~60 Hz to avoid burning CPU on non-GPU windows.
    let wgc_period = Duration::from_millis(4);
    let gdi_period = Duration::from_millis(16);




    // PRIMARY capture path: Windows Graphics Capture (WGC). It snapshots what
    // the window REALLY renders on the GPU (DirectComposition / DXGI swap-chain),
    // which GDI PrintWindow cannot — so Discord/Opera/Electron and DirectX games
    // capture their real content instead of a black or frozen frame. We create
    // one WGC session for this window; if it cannot be created (old Windows,
    // capture refused) we fall back to the GDI PrintWindow path below.
    let mut wgc = super::wgc::WgcCapture::new(hwnd);
    if wgc.is_some() {
        eprintln!("[windowmod] capture_one_window: using WGC for HWND {:?}", hwnd);
    } else {
        eprintln!(
            "[windowmod] capture_one_window: WGC unavailable for HWND {:?}, using PrintWindow",
            hwnd,
        );
    }

    // Reusable GDI capture resources for the PrintWindow fallback. Creating and
    // destroying the DC + bitmap on EVERY frame (60×/sec per window) was pure
    // overhead — GetDC/CreateCompatibleDC/CreateCompatibleBitmap/DeleteObject/
    // DeleteDC/ReleaseDC add up across a dozen windows and stacked with the
    // already-costly PrintWindow blit. `GdiCapture` keeps the DC and bitmap
    // alive across frames and only recreates the bitmap when the window resizes,
    // so each frame does just PrintWindow + GetDIBits.
    let mut gdi = GdiCapture::new();


    loop {
        let frame_start = Instant::now();

        // Exit conditions: the window died, or it was removed from the live set
        // (no longer a capture target). Checked every tick so threads clean up.
        if !hwnd_alive(hwnd) {
            break;
        }
        let still_live = match live_threads().lock() {
            Ok(g) => g.contains(&hwnd_isize),
            Err(_) => false,
        };
        if !still_live {
            break;
        }

        // Capture this frame. `captured` holds (width, height) and the pixels
        // live in `scratch[..w*h*4]` (BGRA, top-down). WGC and PrintWindow both
        // produce that layout.
        let captured: Option<(i32, i32)> = if let Some(cap) = wgc.as_mut() {
            // WGC path: grab the latest composed frame. Returns None when no new
            // frame is ready yet (keep the previous one) — that is normal and
            // means the window content did not change.
            match cap.grab(&mut scratch) {
                Some((w, h)) => Some((w, h)),
                None => {
                    // On hidden desktops DWM may not composite the window
                    // until it is explicitly told to repaint. Nudge it so
                    // WGC gets a real frame instead of staying black.
                    maybe_request_repaint(hwnd_isize);
                    None
                }
            }
        } else {
            // FALLBACK PrintWindow path (non-GPU windows, or WGC unavailable).
            let (w, h) = client_size(hwnd);
            if w < 1 || h < 1 {
                None
            } else {
                let size = (w * h * 4) as usize;
                if scratch.len() < size {
                    scratch.resize(size, 0);
                }
                // Nudge non-GPU windows to repaint (throttled) so a genuinely
                // idle/frozen GDI app still refreshes.
                maybe_request_repaint(hwnd_isize);
                if gdi.capture(hwnd, w, h, &mut scratch[..size]) {
                    Some((w, h))
                } else {
                    None
                }

            }
        };

        if let Some((w, h)) = captured {
            let size = (w as usize) * (h as usize) * 4;
            if scratch.len() >= size && size > 0 {
                let buf = &scratch[..size];
                // Cheap sampled checksum OUTSIDE the lock so we only publish (and
                // allocate) when the window's pixels actually changed — idle
                // windows cost nothing and never force a render-thread re-upload.
                let new_checksum = sampled_checksum(buf);

                let (size_changed, content_changed) = {
                    match frames().lock() {
                        Ok(map) => match map.get(&hwnd_isize) {
                            Some(e) => (
                                e.width != w || e.height != h || e.data.len() != buf.len(),
                                e.checksum != new_checksum,
                            ),
                            None => (true, true),
                        },
                        Err(_) => (true, true),
                    }
                };

                if size_changed || content_changed {
                    // Allocate the published copy OUTSIDE the lock, then move
                    // only an Arc under the lock (no memcpy while holding it).
                    let frame_data = Arc::new(buf.to_vec());
                    if let Ok(mut map) = frames().lock() {
                        let entry = map.entry(hwnd_isize).or_insert(SharedFrame {
                            width: w,
                            height: h,
                            data: Arc::new(Vec::new()),
                            version: 0,
                            checksum: 0,
                        });
                        entry.width = w;
                        entry.height = h;
                        entry.data = frame_data;
                        entry.checksum = new_checksum;
                        entry.version = entry.version.wrapping_add(1);
                    }
                }
            }
        }

        // Pace this window. Only the window the player is CURRENTLY VIEWING (the
        // focused toplevel) needs a live, smooth feed — every other window is
        // off-screen and its pixels are invisible right now, so capturing it
        // 60×/sec with the expensive PrintWindow blit just burns CPU and was the
        // real source of the lag once many apps were open. So:
        //   * WGC path      -> always fast (~240 Hz); cheap null frames.
        //   * GDI focused    -> ~60 Hz (16 ms): smooth for the window on screen.
        //   * GDI background -> ~8 Hz (125 ms): fresh within an eyeblink when the
        //     player switches to it, but ~7× less PrintWindow work.
        let is_focused = FOCUSED_CAPTURE_HWND.load(Ordering::Relaxed) == hwnd_isize;
        let target_period = if wgc.is_some() {
            wgc_period
        } else if is_focused {
            gdi_period
        } else {
            Duration::from_millis(125)
        };
        let elapsed = frame_start.elapsed();

        if let Some(remaining) = target_period.checked_sub(elapsed) {
            std::thread::sleep(remaining);
        }

    }

    // Drop the WGC session (stops capture) before clearing shared state.
    drop(wgc);

    // Thread exiting: drop our published frame and our live-set membership so a
    // future re-registration of the same HWND starts fresh.
    if let Ok(mut map) = frames().lock() {
        map.remove(&hwnd_isize);
    }
    if let Ok(mut live) = live_threads().lock() {
        live.remove(&hwnd_isize);
    }
}





pub fn refresh_windows(state: &mut WindowMod) {
    // 1) Compute the current set of live HWNDs and make sure each one has its
    //    OWN dedicated capture thread (and stale threads are told to exit). One
    //    thread per window is what prevents a single slow/stuck PrintWindow from
    //    freezing the capture of every other window — the "everything freezes
    //    after a while" bug. New windows get a thread immediately; closed ones
    //    have their thread exit on the next tick.
    let live_targets: Vec<isize> = state
        .toplevels
        .iter()
        .filter(|t| hwnd_alive(t.hwnd))
        .map(|t| t.hwnd.0 as isize)
        .collect();
    ensure_per_window_threads(&live_targets);


    // 2) Copy the latest finished frame for each window from the shared map.
    //    This is a cheap memcpy on the render thread — no PrintWindow here.
    let count = state.toplevels.len();
    for i in 0..count {
        let update = {
            let toplevel = &mut state.toplevels[i];
            if !hwnd_alive(toplevel.hwnd) {
                toplevel.mapped = false;
                continue;
            }
            toplevel.mapped = true;

            let hwnd_isize = toplevel.hwnd.0 as isize;
            let frame = match frames().lock() {
                Ok(map) => match map.get(&hwnd_isize) {
                    Some(f) if !f.data.is_empty() => {
                        // Only upload when the version changed since last copy.
                        if f.version == toplevel.last_frame_version {
                            None
                        } else {
                            Some((f.width, f.height, f.data.clone(), f.version))
                        }
                    }
                    _ => None,
                },
                Err(_) => None,
            };

            let Some((w, h, data, version)) = frame else {
                continue;
            };

            if w != toplevel.width || h != toplevel.height {
                toplevel.width = w;
                toplevel.height = h;
                toplevel.geom_w = w;
                toplevel.geom_h = h;
            }
            // SHARE the frame by moving the Arc — an atomic refcount bump — instead
            // of memcpy'ing ~8 MB into a private buffer on the render thread. The
            // capture thread's data is immutable behind the Arc, and Java reads the
            // pixels synchronously in update_surface_data before the next frame
            // replaces this Arc, so pointing at the shared buffer is safe.
            toplevel.buffer = data;
            toplevel.last_frame_version = version;

            let is_zero = toplevel.buffer.iter().take(64).all(|&b| b == 0);

            if is_zero {
                toplevel.zero_capture_count += 1;
            } else {
                toplevel.zero_capture_count = 0;
            }

            // NOTE: window_title() sends a synchronous WM_GETTEXT to the target
            // window. For a busy/blocking app (browser) that call can stall for
            // hundreds of ms — and it ran every frame for every window, which is
            // what spiked native update() to ~700-1400 ms. Refresh the title
            // only occasionally instead.
            let ptr = ptr_of(toplevel.as_mut());
            Some((ptr, w, h))

        };

        if let Some((toplevel_ptr, w, h)) = update {
            for surface in state.surfaces.iter_mut() {
                if surface.toplevel_ptr == toplevel_ptr {
                    surface.buffer_dirty = true;
                    surface.damage.push([0, 0, w, h]);
                }
            }
        }
    }
}


pub fn hwnd_alive(hwnd: HWND) -> bool {


    unsafe { IsWindow(hwnd).as_bool() }
}

/// Ask a window to repaint its whole client area WITHOUT blocking on it.
///
/// Some apps stop repainting their window once it is off the visible desktop or
/// after a child process they were hosting (a launched game) exits — e.g. Steam
/// "freezes" after you close a game launched through it. PrintWindow then keeps
/// returning the stale last frame. `RedrawWindow` with RDW_INVALIDATE only marks
/// the area dirty and POSTS a WM_PAINT; it does not wait for the app to process
/// it (that would be RDW_UPDATENOW), so a genuinely hung app can never stall our
/// capture thread. RDW_ALLCHILDREN propagates to child render widgets so
/// Chromium/SDL surfaces refresh too.
fn request_repaint(hwnd: HWND) {
    use windows::Win32::Graphics::Gdi::{
        RedrawWindow, RDW_ALLCHILDREN, RDW_INVALIDATE,
    };
    unsafe {
        let _ = RedrawWindow(hwnd, None, None, RDW_INVALIDATE | RDW_ALLCHILDREN);
    }
}

/// Per-window timestamp of the last repaint nudge, so `maybe_request_repaint`
/// can throttle to at most one invalidate every `REPAINT_INTERVAL`.
static LAST_REPAINT: OnceLock<Mutex<HashMap<isize, Instant>>> = OnceLock::new();

fn last_repaint() -> &'static Mutex<HashMap<isize, Instant>> {
    LAST_REPAINT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Minimum gap between repaint nudges for one window. ~500 ms is frequent
/// enough to un-stick a frozen app within half a second, yet rare enough that
/// normal animating windows are driven by their OWN repaints (not ours), so the
/// change-detection checksum still skips truly-static windows.
const REPAINT_INTERVAL: Duration = Duration::from_millis(500);

/// Throttled wrapper around `request_repaint`: invalidates a window at most once
/// every `REPAINT_INTERVAL`. Invalidating every capture pass (60-120×/sec) made
/// every window report "changed" each frame, forcing a full-buffer memcpy per
/// window on the render thread and spiking native update() to 56-76 ms. This
/// keeps the un-freeze nudge while letting static windows stay static.
fn maybe_request_repaint(hwnd_isize: isize) {
    let now = Instant::now();
    let due = {
        match last_repaint().lock() {
            Ok(mut map) => {
                let last = map.get(&hwnd_isize).copied();
                let due = last.map(|t| now.duration_since(t) >= REPAINT_INTERVAL).unwrap_or(true);
                if due {
                    map.insert(hwnd_isize, now);
                }
                due
            }
            Err(_) => true,
        }
    };
    if due {
        request_repaint(HWND(hwnd_isize as *mut _));
    }
}



fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = windows::Win32::Foundation::RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

// InternalGetWindowText reads the window's title from the kernel-cached copy
// WITHOUT sending WM_GETTEXT to the window's thread. Plain GetWindowTextW sends
// a SYNCHRONOUS WM_GETTEXT, which BLOCKS until the target thread pumps it — for
// a busy app (Opera playing a video, Steam, a loading game) that stalled the
// render thread for ~1.5 SECONDS every time we read a title, which the log
// showed as `native update() took 1565 ms` spikes (the game froze ~1×/2 s).
// InternalGetWindowText never touches the target thread, so it can never block.
#[link(name = "user32")]
extern "system" {
    fn InternalGetWindowText(hwnd: HWND, psz: *mut u16, cch: i32) -> i32;
}

fn window_title(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let read = InternalGetWindowText(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if read <= 0 {
            String::new()
        } else {
            String::from_utf16_lossy(&buf[..read as usize])
        }
    }
}


/// Per-capture-thread GDI resources for the PrintWindow fallback path, reused
/// across frames. Creating and destroying a memory DC + bitmap every frame was
/// significant overhead once a dozen windows were open; keeping them alive and
/// recreating only on a size change means each frame does just PrintWindow +
/// GetDIBits.
struct GdiCapture {
    /// Cached memory DC (created once, compatible with the screen). None until
    /// the first successful capture.
    mem_dc: Option<HDC>,
    /// Cached DIB section we PrintWindow into and GetDIBits out of.
    bitmap: Option<windows::Win32::Graphics::Gdi::HBITMAP>,
    /// The GDI object that was selected into `mem_dc` before we put our bitmap
    /// in, so we can restore it before deleting the DC.
    old_obj: HGDIOBJ,
    /// Size the cached bitmap was created for; recreated when the window resizes.
    bw: i32,
    bh: i32,
}

impl GdiCapture {
    fn new() -> GdiCapture {
        GdiCapture {
            mem_dc: None,
            bitmap: None,
            old_obj: HGDIOBJ(std::ptr::null_mut()),
            bw: 0,
            bh: 0,
        }
    }

    /// Capture `hwnd`'s client area into `out` (BGRA, top-down). Reuses the
    /// cached DC/bitmap when the size is unchanged.
    fn capture(&mut self, hwnd: HWND, width: i32, height: i32, out: &mut [u8]) -> bool {
        unsafe {
            let hdc_window = GetDC(hwnd);
            if hdc_window.is_invalid() {
                return false;
            }

            // Ensure the memory DC exists (created once, reused across frames).
            if self.mem_dc.is_none() {
                let dc = CreateCompatibleDC(hdc_window);
                if dc.is_invalid() {
                    let _ = ReleaseDC(hwnd, hdc_window);
                    return false;
                }
                self.mem_dc = Some(dc);
            }
            let mem_dc = self.mem_dc.unwrap();

            // (Re)create the bitmap only when the window size changed.
            if self.bitmap.is_none() || self.bw != width || self.bh != height {
                // Restore the previous object and free the old bitmap first.
                if let Some(old_bmp) = self.bitmap.take() {
                    SelectObject(mem_dc, self.old_obj);
                    let _ = DeleteObject(HGDIOBJ(old_bmp.0));
                }
                let hbm = CreateCompatibleBitmap(hdc_window, width, height);
                if hbm.is_invalid() {
                    let _ = ReleaseDC(hwnd, hdc_window);
                    return false;
                }
                self.old_obj = SelectObject(mem_dc, HGDIOBJ(hbm.0));
                self.bitmap = Some(hbm);
                self.bw = width;
                self.bh = height;
            }
            let hbm = self.bitmap.unwrap();

            // Capture the CLIENT area only (see PW_CLIENTONLY note below). Without
            // it, PrintWindow renders the whole window frame and shifts client
            // content down, misaligning clicks. PW_RENDERFULLCONTENT keeps
            // DirectComposition/Chromium surfaces rendering their content.
            let printed =
                PrintWindow(hwnd, mem_dc, PW_CLIENTONLY | PW_RENDERFULLCONTENT).as_bool();

            let ok = if printed {
                read_bitmap_bgra(mem_dc, hbm, width, height, out)
            } else {
                false
            };

            let _ = ReleaseDC(hwnd, hdc_window);
            ok
        }
    }
}

impl Drop for GdiCapture {
    fn drop(&mut self) {
        unsafe {
            if let Some(mem_dc) = self.mem_dc {
                if let Some(bmp) = self.bitmap.take() {
                    SelectObject(mem_dc, self.old_obj);
                    let _ = DeleteObject(HGDIOBJ(bmp.0));
                }
                let _ = DeleteDC(mem_dc);
            }
        }
    }
}


unsafe fn read_bitmap_bgra(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    hbm: windows::Win32::Graphics::Gdi::HBITMAP,
    width: i32,
    height: i32,
    out: &mut [u8],
) -> bool {
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let stride = width * 4;
    let size = (stride * height) as usize;
    if out.len() < size {
        return false;
    }

    GetDIBits(
        hdc,
        hbm,
        0,
        height as u32,
        Some(out.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    ) > 0
}

pub fn find_main_window_for_pid(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        let mut window_pid = 0u32;
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            hwnd,
            Some(&mut window_pid),
        );
        if window_pid == search.pid {
            // Skip WS_CHILD (message-only, etc.)
            let style = GetWindowLongW(hwnd, GWL_STYLE);
            if style & WS_CHILD.0 as i32 == 0 {
                search.found = Some(hwnd);
                return BOOL(0); // stop enumeration — found
            }
        }
        TRUE
    }

    let mut search = Search { pid, found: None };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}

pub fn register_external_hwnd(
    state: &mut WindowMod,
    hwnd: HWND,
    app_id: String,
) -> Option<i64> {
    if !hwnd_alive(hwnd) {
        eprintln!("[windowmod] register_external_hwnd: hwnd {:?} not alive", hwnd);
        return None;
    }

    // Skip if already registered
    if state.toplevels.iter().any(|t| t.hwnd == hwnd) {
        eprintln!("[windowmod] register_external_hwnd: hwnd {:?} already registered", hwnd);
        return None;
    }

    let (w, h) = client_size(hwnd);
    if w < 10 || h < 10 {
        eprintln!("[windowmod] register_external_hwnd: skipping HWND {:?} — client_size {}x{} < 10x10", hwnd, w, h);
        return None;
    }
    let title = window_title(hwnd);

    let toplevel = WinToplevel {
        hwnd,
        title,
        app_id,
        width: w.max(640),
        height: h.max(480),
        geom_x: 0,
        geom_y: 0,
        geom_w: w.max(640),
        geom_h: h.max(480),
        buffer: Arc::new(vec![0; (w.max(640) * h.max(480) * 4) as usize]),
        mapped: true,

        maximize: false,
        fullscreen: false,
        requests: Default::default(),
        zero_capture_count: 0,
        prev_first16: [0u8; 16],
        static_frame_count: 0,
        last_frame_version: 0,
        is_popup: false,
        owner_hwnd: HWND(std::ptr::null_mut()),
    };


    let toplevel_ptr = state.insert_toplevel(toplevel);
    eprintln!(
        "[windowmod]   inserted toplevel at ptr={}, toplevels now {}",
        toplevel_ptr,
        state.toplevels.len(),
    );

    state.insert_surface(WinSurface {
        toplevel_ptr,
        parent_ptr: 0,
        xoff: 0,
        yoff: 0,
        damage: vec![[0, 0, w.max(640), h.max(480)]],
        buffer_dirty: true,
    });
    eprintln!(
        "[windowmod]   inserted surface for ptr={}, surfaces now {}",
        toplevel_ptr,
        state.surfaces.len(),
    );

    eprintln!("[windowmod] register_external_hwnd: registered HWND {:?} -> ptr={}", hwnd, toplevel_ptr);
    Some(toplevel_ptr)
}

/// Re-point an EXISTING toplevel (identified by its native pointer) at a NEW
/// HWND, updating its app_id/title and resetting its capture buffer. Returns
/// true on success.
///
/// This is the heart of "process tracking": when a launcher (TLauncher, Steam,
/// an installer) or a self-relaunching app (Electron splash → main window)
/// replaces its window with a different one — possibly in a child process — we
/// keep the SAME toplevel pointer and just retarget it. Because the player's
/// window-item stores that pointer, the item seamlessly starts showing the new
/// window (e.g. the TLauncher item becomes the launched Minecraft) without the
/// item ever going invalid.
pub fn reassign_toplevel_hwnd(
    state: &mut WindowMod,
    toplevel_ptr: i64,
    new_hwnd: HWND,
    new_app_id: String,
) -> bool {
    if !hwnd_alive(new_hwnd) {
        return false;
    }
    // Don't adopt a HWND that's already tracked by another toplevel.
    if state
        .toplevels
        .iter()
        .any(|t| t.hwnd == new_hwnd && ptr_of_const(&**t) != toplevel_ptr)
    {
        return false;
    }

    // CRITICAL: the toplevel a PendingLaunch points at may already have been
    // freed by `retain_toplevels` (e.g. a launcher/splash window was destroyed
    // before its successor appeared). The raw `toplevel_ptr` then dangles, and
    // dereferencing it via `ptr_to_mut` is use-after-free — which previously
    // crashed the game with a "non-string panic payload". Only proceed if the
    // pointer still corresponds to a LIVE toplevel owned by `state.toplevels`.
    // Otherwise return false so the caller falls back to registering a fresh
    // toplevel.
    if !state
        .toplevels
        .iter()
        .any(|t| ptr_of_const(&**t) == toplevel_ptr)
    {
        eprintln!(
            "[windowmod] reassign_toplevel_hwnd: ptr={} no longer live, falling back to fresh register",
            toplevel_ptr,
        );
        return false;
    }

    let Some(t) = super::state::ptr_to_mut::<WinToplevel>(toplevel_ptr) else {
        return false;
    };

    let (w, h) = client_size(new_hwnd);
    if w < 10 || h < 10 {
        return false;
    }
    let title = window_title(new_hwnd);
    eprintln!(
        "[windowmod] reassign_toplevel_hwnd: ptr={} {:?} -> {:?} app_id='{}' {}x{}",
        toplevel_ptr, t.hwnd, new_hwnd, new_app_id, w, h,
    );
    t.hwnd = new_hwnd;
    t.app_id = new_app_id;
    t.title = title;
    t.mapped = true;
    t.width = w.max(640);
    t.height = h.max(480);
    t.geom_w = w.max(640);
    t.geom_h = h.max(480);
    t.last_frame_version = 0;
    // Mark the owning surface dirty so the new window is re-captured/redrawn.
    for s in state.surfaces.iter_mut() {
        if s.toplevel_ptr == toplevel_ptr {
            s.buffer_dirty = true;
            s.damage.push([0, 0, t.geom_w, t.geom_h]);
        }
    }
    true
}

fn ptr_of_const(t: &WinToplevel) -> i64 {
    (t as *const WinToplevel) as i64
}

/// Register a popup window (dropdown/context menu, combo list, flyout) so it

/// renders INSIDE the owning toplevel window as a child surface — like a real
/// desktop dropdown — instead of as a separate floating Minecraft window.
///
/// Unlike `register_external_hwnd`, this:
///   * accepts small windows (menus are small),
///   * creates an `is_popup` toplevel (captured like any window, but excluded
///     from the JNI `toplevels()` list),
///   * links the popup's surface as a CHILD of `owner_surface_ptr` with the
///     popup's pixel offset relative to the owner window's client origin.
///
/// `owner_hwnd` is the owning toplevel's HWND (its client origin is the popup
/// offset reference); `owner_surface_ptr` is the owner toplevel's root surface.
pub fn register_popup_hwnd(
    state: &mut WindowMod,
    hwnd: HWND,
    owner_hwnd: HWND,
    owner_surface_ptr: i64,
) -> Option<i64> {
    if !hwnd_alive(hwnd) {
        return None;
    }
    if state.toplevels.iter().any(|t| t.hwnd == hwnd) {
        return None;
    }

    let (w, h) = client_size(hwnd);
    if w < 1 || h < 1 {
        return None;
    }

    let (xoff, yoff) = popup_offset_in_owner(hwnd, owner_hwnd);

    let toplevel = WinToplevel {
        hwnd,
        title: String::new(),
        app_id: "popup".to_string(),
        width: w,
        height: h,
        geom_x: 0,
        geom_y: 0,
        geom_w: w,
        geom_h: h,
        buffer: Arc::new(vec![0; (w * h * 4) as usize]),
        mapped: true,

        maximize: false,
        fullscreen: false,
        requests: Default::default(),
        zero_capture_count: 0,
        prev_first16: [0u8; 16],
        static_frame_count: 0,
        last_frame_version: 0,
        is_popup: true,
        owner_hwnd,
    };

    let toplevel_ptr = state.insert_toplevel(toplevel);

    // The popup's surface uses its OWN popup-toplevel for its captured buffer,
    // but is parented to the OWNER's root surface so the surface-tree walk on
    // the Java side draws it inside the owner's window at (xoff, yoff).
    state.insert_surface(WinSurface {
        toplevel_ptr,
        parent_ptr: owner_surface_ptr,
        xoff,
        yoff,
        damage: vec![[0, 0, w, h]],
        buffer_dirty: true,
    });

    eprintln!(
        "[windowmod] register_popup_hwnd: HWND {:?} -> ptr={} offset=({},{}) {}x{} (child of surface 0x{:x})",
        hwnd, toplevel_ptr, xoff, yoff, w, h, owner_surface_ptr,
    );
    Some(toplevel_ptr)
}

/// Compute the popup's top-left position in the owner window's CLIENT
/// coordinates. Both windows live on the hidden desktop; their screen
/// rectangles are virtual but consistent, so subtracting the owner's client
/// origin yields a correct relative offset for in-window rendering.
fn popup_offset_in_owner(popup: HWND, owner: HWND) -> (i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    unsafe {
        // Owner client origin in screen coords.
        let mut owner_origin = POINT { x: 0, y: 0 };
        let _ = ClientToScreen(owner, &mut owner_origin);

        // Popup top-left in screen coords (popups are top-level windows, so
        // GetWindowRect gives their on-screen rectangle).
        let mut popup_rect = RECT::default();
        let _ = GetWindowRect(popup, &mut popup_rect);

        (popup_rect.left - owner_origin.x, popup_rect.top - owner_origin.y)
    }
}

/// Refresh a popup surface's offset relative to its owner each frame (the menu
/// may not move, but the owner could be repositioned/resized). Returns the new
/// offset.
pub fn refresh_popup_offset(popup: HWND, owner: HWND) -> (i32, i32) {
    popup_offset_in_owner(popup, owner)
}

