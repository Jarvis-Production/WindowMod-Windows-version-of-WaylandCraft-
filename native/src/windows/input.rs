use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VSC_TO_VK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindow, GetWindowTextW, GetWindowLongW, GetWindowThreadProcessId, IsWindowVisible,
    PostMessageW, SendMessageW, ShowWindow, SW_MINIMIZE, GW_CHILD, GW_HWNDNEXT, GWL_STYLE,
    WS_VISIBLE, WM_ACTIVATE, WM_CHAR, WM_HSCROLL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCHITTEST, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, HTCLIENT, MA_ACTIVATE, WA_ACTIVE,
};




use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::Foundation::{LPARAM, HWND, POINT, RECT, WPARAM};






use super::state::{ptr_to_mut, ptr_to_ref, WindowMod};

// Declared directly against user32 — not all of these are exposed by the
// enabled `windows` crate features in this version.
#[link(name = "user32")]
extern "system" {
    fn SetForegroundWindow(hwnd: HWND) -> i32;
    fn ToUnicodeEx(
        uVirtKey: u32, uScanCode: u32, lpKeyState: *const u8,
        pwszBuff: *mut u16, cchBuff: u32, wFlags: u32, dwhkl: isize,
    ) -> i32;
    fn GetKeyboardLayout(idThread: u32) -> isize;
    fn GetAsyncKeyState(vKey: i32) -> i16;
    fn GetKeyState(nVKey: i32) -> i16;
    fn GetGUIThreadInfo(idThread: u32, pgui: *mut GuiThreadInfo) -> i32;
    fn GetClassNameW(hwnd: HWND, lpClassName: *mut u16, nMaxCount: i32) -> i32;
    // Not exposed by the enabled `windows` crate features in this version.
    // Translates `cPoints` POINTs from `hWndFrom`'s coordinate space into
    // `hWndTo`'s, using only the relative offset between the two windows — so
    // it is correct even for windows parked far off-screen.
    fn MapWindowPoints(
        hWndFrom: HWND, hWndTo: HWND, lpPoints: *mut POINT, cPoints: u32,
    ) -> i32;
}


pub fn class_name(hwnd: HWND) -> String {

    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..len as usize])
    }
}


#[repr(C)]
#[derive(Clone, Copy)]
struct GuiThreadInfo {
    cb_size: u32,
    flags: u32,
    hwnd_active: HWND,
    hwnd_focus: HWND,
    hwnd_capture: HWND,
    hwnd_menu_owner: HWND,
    hwnd_move_size: HWND,
    hwnd_caret: HWND,
    rc_caret: RECT,
}

static mut MC_HWND: isize = 0;

pub fn set_win32_hwnd(hwnd: isize) {
    unsafe { MC_HWND = hwnd; }
}

fn build_keyboard_state() -> [u8; 256] {
    let mut state = [0u8; 256];
    unsafe {
        for vk in 0..256i32 {
            if GetAsyncKeyState(vk) as u16 & 0x8000 != 0 {
                state[vk as usize] |= 0x80;
            }
            if GetKeyState(vk) as u16 & 0x8000 != 0 {
                state[vk as usize] |= 0x80;
            }
        }
    }
    state
}

/// Hit-test the direct child windows of `parent` at the point `local` (given in
/// `parent`'s CLIENT coordinates). Returns the topmost visible child that
/// contains the point, plus the point translated into that child's client
/// coordinates. Returns `None` if no child contains the point.
///
/// This deliberately does NOT use `ChildWindowFromPointEx` / `WindowFromPoint`,
/// because those perform SCREEN-based hit testing. Our target windows are
/// parked far off-screen (≈ -32000,-32000), where screen hit testing returns
/// garbage. Instead we enumerate children via `GetWindow` and translate
/// coordinates with `MapWindowPoints`, which works purely on relative offsets
/// and is therefore correct regardless of the window's absolute position.
fn hit_test_child(parent: HWND, local: POINT) -> Option<(HWND, POINT)> {
    unsafe {
        // Children are returned in Z-order (topmost first via GW_CHILD, then
        // GW_HWNDNEXT walks down the Z-order). The first visible child that
        // contains the point is the one that would receive the mouse message.
        let mut child = GetWindow(parent, GW_CHILD).unwrap_or(HWND(std::ptr::null_mut()));
        while !child.0.is_null() {
            // Skip invisible children — they never receive mouse input.
            let visible = IsWindowVisible(child).as_bool()
                || (GetWindowLongW(child, GWL_STYLE) & WS_VISIBLE.0 as i32) != 0;
            if visible {
                // Translate the point from `parent`'s client space into
                // `child`'s client space. MapWindowPoints adjusts by the delta
                // between the two windows' client origins (relative, so correct
                // even off-screen).
                let mut pt = local;
                MapWindowPoints(parent, child, &mut pt, 1);


                // Get the child's client rectangle (origin always 0,0).
                let mut rc = RECT::default();
                if windows::Win32::UI::WindowsAndMessaging::GetClientRect(child, &mut rc).is_ok()
                    && pt.x >= rc.left
                    && pt.x < rc.right
                    && pt.y >= rc.top
                    && pt.y < rc.bottom
                {
                    return Some((child, pt));
                }
            }
            child = GetWindow(child, GW_HWNDNEXT).unwrap_or(HWND(std::ptr::null_mut()));
        }
    }
    None
}

/// Resolve the real child window under a client-space point of `top` and
/// translate the point into that child's client coordinates.
///
/// Mouse messages must be delivered to the deepest child window under the
/// cursor (a button, an edit control, a render widget, ...). Posting them to
/// the top-level HWND does nothing because Windows mouse dispatch walks the
/// child window tree itself — synthesized messages do not.
fn target_child_at(top: HWND, cx: i32, cy: i32) -> (HWND, i32, i32) {
    // `cx,cy` arrive in `top`'s client coordinates. Walk down the child window
    // tree using our own off-screen-safe hit test, keeping the point in the
    // current window's client coordinates at every step.
    let mut current = top;
    let mut local = POINT { x: cx, y: cy };

    // Bound the depth so a pathological window tree can never loop forever.
    for _ in 0..32 {
        match hit_test_child(current, local) {
            Some((child, child_pt)) if child != current => {
                current = child;
                local = child_pt;
            }
            _ => break,
        }
    }

    (current, local.x, local.y)
}

/// Find the first descendant window of `root` whose class name is exactly
/// `wanted`, returning it together with the point `(cx,cy)` (given in `root`'s
/// client coordinates) translated into that descendant's client coordinates.
///
/// Chromium (Opera/Chrome/Discord/VS Code) renders web content into a
/// `Chrome_RenderWidgetHostHWND`, but composites it through an
/// `Intermediate D3D Window` that sits ABOVE the render widget in Z-order. Our
/// hit test therefore lands on the D3D layer — which is a pure GPU surface that
/// IGNORES every synthesized WM_MOUSE*/WM_CHAR message, so clicks and typing
/// went nowhere ("can't click or type in Opera"). The widget that actually
/// CONSUMES synthesized input is `Chrome_RenderWidgetHostHWND`. We locate it by
/// walking the child tree and redirect input there, keeping the click point
/// correct by re-basing it into the widget's client space.
fn find_descendant_by_class(root: HWND, wanted: &str, cx: i32, cy: i32) -> Option<(HWND, i32, i32)> {
    unsafe {
        // Breadth-first walk of the whole child window tree.
        let mut queue: Vec<HWND> = vec![root];
        let mut visited = 0;
        while let Some(parent) = queue.pop() {
            visited += 1;
            if visited > 256 {
                break;
            }
            let mut child = GetWindow(parent, GW_CHILD).unwrap_or(HWND(std::ptr::null_mut()));
            while !child.0.is_null() {
                if class_name(child) == wanted {
                    // Translate the root-relative point into the widget's client
                    // coordinates so the click lands at the right pixel.
                    let mut pt = POINT { x: cx, y: cy };
                    MapWindowPoints(root, child, &mut pt, 1);
                    return Some((child, pt.x, pt.y));
                }
                queue.push(child);
                child = GetWindow(child, GW_HWNDNEXT).unwrap_or(HWND(std::ptr::null_mut()));
            }
        }
    }
    None
}

/// For a Chromium top-level window, resolve the real input-consuming widget
/// (`Chrome_RenderWidgetHostHWND`) and the point in its client coordinates.
/// Returns None for non-Chromium windows or when the widget cannot be found.
/// `cx,cy` are in `top`'s client coordinates.
fn chromium_input_target(top: HWND, cx: i32, cy: i32) -> Option<(HWND, i32, i32)> {
    let top_cls = class_name(top);
    if top_cls != "Chrome_WidgetWin_1" && top_cls != "Chrome_WidgetWin_0" {
        return None;
    }
    find_descendant_by_class(top, "Chrome_RenderWidgetHostHWND", cx, cy)
}

/// True if `top` is a Chromium/Electron top-level window (Opera, Chrome,
/// Discord, VS Code). These accept synthesized WM_CHAR/WM_KEYDOWN directly once
/// their accessibility tree is awake, so the UIA layer must NEVER call
/// `SetFocus` on them — doing so steals the system input focus, GLFW reports a
/// focus change on Minecraft and the captured keyboard stops delivering keys
/// (the "Opera types for ~5 seconds then stops" freeze).
#[allow(dead_code)]
fn is_chromium_top(top: HWND) -> bool {
    let cls = class_name(top);
    cls == "Chrome_WidgetWin_1" || cls == "Chrome_WidgetWin_0"
}






/// Find which window currently holds keyboard focus *inside the target
/// application's own thread*, without changing the global Windows focus.
///
/// This is the key to typing into an offscreen window without stealing focus
/// from Minecraft (stealing focus made Minecraft think its GUI lost focus and
/// triggered ESC). Returns the focused HWND, or `top` as a fallback.
fn thread_focus_window(top: HWND) -> HWND {
    unsafe {
        let thread = GetWindowThreadProcessId(top, None);
        if thread == 0 {
            return top;
        }
        let mut info = GuiThreadInfo {
            cb_size: std::mem::size_of::<GuiThreadInfo>() as u32,
            flags: 0,
            hwnd_active: HWND(std::ptr::null_mut()),
            hwnd_focus: HWND(std::ptr::null_mut()),
            hwnd_capture: HWND(std::ptr::null_mut()),
            hwnd_menu_owner: HWND(std::ptr::null_mut()),
            hwnd_move_size: HWND(std::ptr::null_mut()),
            hwnd_caret: HWND(std::ptr::null_mut()),
            rc_caret: RECT::default(),
        };
        if GetGUIThreadInfo(thread, &mut info) != 0 && !info.hwnd_focus.0.is_null() {
            info.hwnd_focus
        } else if !info.hwnd_active.0.is_null() {
            info.hwnd_active
        } else {
            top
        }
    }
}

/// Make a window on the (hidden) desktop believe it is the active, focused
/// window so it accepts synthesized keyboard/mouse input.
///
/// On a hidden desktop nothing ever calls SetForegroundWindow/SetFocus on these
/// windows, so edit controls keep a blinking-less, inactive caret and ignore
/// WM_CHAR, and many controls ignore clicks until activated. We synthesize the
/// activation handshake (WM_MOUSEACTIVATE + WM_ACTIVATE + WM_SETFOCUS) WITHOUT
/// touching the global foreground window — so Minecraft keeps its own focus.
///
/// `top` is the registered top-level window; `target` is the deepest child that
/// will receive the actual input message.
fn activate_target(top: HWND, target: HWND) {
    unsafe {
        // Tell the top-level it is being activated by a mouse click in its
        // client area (does not change global Z-order / foreground).
        let _ = SendMessageW(
            top,
            WM_MOUSEACTIVATE,
            WPARAM(top.0 as usize),
            LPARAM(((WM_LBUTTONDOWN as u32) << 16 | HTCLIENT as u32) as isize),
        );
        // Activate the top-level window (wParam low word = WA_ACTIVE).
        let _ = SendMessageW(top, WM_ACTIVATE, WPARAM(WA_ACTIVE as usize), LPARAM(0));
        // Give keyboard focus to the deepest child control.
        let _ = SendMessageW(target, WM_SETFOCUS, WPARAM(0), LPARAM(0));
        // Keep the linker happy referencing these even if some paths skip them.
        let _ = (MA_ACTIVATE, WM_NCHITTEST);
    }
}

pub fn pointer_motion(state: &mut WindowMod, x: f64, y: f64) {
    state.pointer_x = x;
    state.pointer_y = y;
    if state.pointer_focus_ptr != 0 {
        send_mouse_move(state, state.pointer_focus_ptr, x, y);
    }
}

pub fn pointer_motion_focus(state: &mut WindowMod, surface_ptr: i64, x: f64, y: f64) {
    state.pointer_focus_ptr = surface_ptr;
    state.pointer_x = x;
    state.pointer_y = y;
    send_mouse_move(state, surface_ptr, x, y);
}

pub fn pointer_leave(state: &mut WindowMod) {
    state.pointer_focus_ptr = 0;
}

pub fn pointer_button(state: &mut WindowMod, button: u32, pressed: bool) -> u32 {
    let serial = state.next_serial();
    if state.pointer_focus_ptr == 0 {
        eprintln!("[windowmod] pointer_button: pointer_focus_ptr=0, no hover");
        return serial;
    }
    let Some(surface) = ptr_to_ref::<super::state::WinSurface>(state.pointer_focus_ptr) else {
        return serial;
    };
    let Some(toplevel) = ptr_to_ref::<super::state::WinToplevel>(surface.toplevel_ptr) else {
        return serial;
    };

    let (msg_down, msg_up, mk_flag) = match button {
        0x110 => (WM_LBUTTONDOWN, WM_LBUTTONUP, 0x0001u32), // MK_LBUTTON
        0x111 => (WM_RBUTTONDOWN, WM_RBUTTONUP, 0x0002u32), // MK_RBUTTON
        0x112 => (WM_MBUTTONDOWN, WM_MBUTTONUP, 0x0010u32), // MK_MBUTTON
        _ => return serial,
    };

    // Track left-button hold state so drags (text selection) carry MK_LBUTTON
    // in subsequent WM_MOUSEMOVE messages, and so the UIA Invoke path can be
    // suppressed for drags (a drag is a selection gesture, not an activation).
    if button == 0x110 {
        state.left_button_down = pressed;
    }

    let msg = if pressed { msg_down } else { msg_up };
    let top = toplevel.hwnd;

    let (mut target, mut lx, mut ly) =
        target_child_at(top, state.pointer_x as i32, state.pointer_y as i32);

    // CHROMIUM redirect: the hit test lands on the `Intermediate D3D Window`
    // (a GPU compositor layer that ignores synthesized input). Redirect mouse
    // messages to the real `Chrome_RenderWidgetHostHWND`, which consumes them.
    // This is what makes clicking inside Opera/Chrome web content work.
    if class_name(target) == "Intermediate D3D Window" {
        if let Some((widget, wx, wy)) =
            chromium_input_target(top, state.pointer_x as i32, state.pointer_y as i32)
        {
            target = widget;
            lx = wx;
            ly = wy;
        }
    }
    let lparam = pack_coords_i(lx, ly);

    // While the button is held, WM_MOUSEMOVE must carry the button's MK_ flag
    // so the control knows the button is down (needed for proper click/drag).
    let move_wparam = if pressed { WPARAM(mk_flag as usize) } else { WPARAM(0) };
    let wparam = WPARAM(mk_flag as usize);

    // Classify the TARGET control (the deepest child under the cursor) to decide
    // whether the UI Automation Invoke path should run IN ADDITION to the
    // PostMessage path.
    //
    // Background:
    //   * Modern renderers (WinUI 3 / UWP / Chromium) IGNORE synthesized
    //     PostMessage mouse messages, so they MUST be driven through UIA Invoke.
    //   * Classic Win32 controls (list views in Save dialogs, desktop icons,
    //     Notepad's open-file list) DO respond to PostMessage. For LIST-type
    //     controls, additionally calling UIA Invoke fired the action a SECOND
    //     time — UIA Invoke on a ListItem "opens" it and the PostMessage click
    //     also opens it — which is what opened 40+ Notepad windows.
    //
    // So we run the UIA Invoke path for everything EXCEPT classic list/tree/
    // header controls, where PostMessage alone is correct and UIA would double-
    // fire. This keeps the previously-working behavior for buttons, menus and
    // modern apps while removing the double-activation on file lists.
    let target_cls = class_name(target);
    // NOTE: `DirectUIHWND` was previously treated as a classic list, which
    // SUPPRESSED the UIA path for it. But `DirectUIHWND` is the Direct-UI host
    // that File Explorer (and common file dialogs) use to draw their item view;
    // it is NOT a real `SysListView32` and frequently IGNORES a synthesized
    // single WM_LBUTTONDOWN, so PostMessage alone never selected/opened items
    // ("can't click anything in Explorer"). UIA Invoke/Select on the underlying
    // ListItem element DOES work there, so we no longer classify it as a classic
    // list — letting the UIA path run for Explorer's item view.
    //
    // `SysTreeView32` was ALSO removed from this list: File Explorer's LEFT
    // navigation pane (the folder tree) is a `SysTreeView32`, and on Windows 11
    // a single synthesized WM_LBUTTONDOWN frequently does NOT select/expand a
    // tree item — so PostMessage alone made the left pane completely unclickable
    // ("mouse doesn't work in the left part of Explorer"). UIA Select/Expand on
    // the underlying TreeItem element DOES work there, and because a tree item
    // exposes SelectionItem/ExpandCollapse (not Invoke), the UIA path does the
    // right single action without the double-fire that plagued real list views.
    let is_classic_list = target_cls == "SysListView32"
        || target_cls == "SysHeader32"
        || target_cls == "ListBox"
        || target_cls.starts_with("WindowsForms10.SysListView");


    // TEXT/EDIT controls. These accept PostMessage mouse input directly and use
    // it for caret placement and click-and-drag SELECTION. Running UIA Invoke on
    // them is wrong twice over:
    //   * Invoke "activates" the control instead of placing the caret/selecting,
    //     which BREAKS click-and-drag text selection entirely.
    //   * On Notepad's recent-files / list-style surfaces it double-fired the
    //     open action together with the PostMessage click — that is what spawned
    //     40+ Notepad windows.
    // Notepad on Windows 11 hosts its editor in `RichEditD2DPT`; classic edit
    // boxes are `Edit`; rich edit controls are `RICHEDIT*`/`RichEdit*`.
    let is_text_edit = target_cls == "Edit"
        || target_cls == "RichEditD2DPT"
        || target_cls.starts_with("RICHEDIT")
        || target_cls.starts_with("RichEdit")
        || target_cls == "Scintilla";

    // CHROMIUM render/composition surface. A Chromium/Electron app (Opera,
    // Chrome, Discord) hosts its web content in `Chrome_RenderWidgetHostHWND`
    // and composites video/GPU layers through an `Intermediate D3D Window`. The
    // log showed that for most of the Opera window `target_child_at` lands on
    // the `Intermediate D3D Window` (it sits ABOVE the render widget in Z-order),
    // which exposes NO actionable UIA element — so we previously skipped UIA for
    // it and the click went nowhere (Chromium ignores PostMessage). That is why
    // "nothing in Opera is clickable".
    //
    // The fix: for Chromium surfaces we STILL run UIA, but resolve the element
    // against the TOP-LEVEL window (Chrome_WidgetWin_1), whose automation tree
    // contains the REAL web content. `deepest_at` then walks down to the element
    // under the cursor by screen coordinates regardless of which child HWND the
    // pixel-level hit test happened to return. So we no longer suppress UIA here.

    // Should UIA be resolved against the TOP-LEVEL window rather than the
    // deepest child? Yes for self-rendering apps whose actionable automation
    // tree hangs off the top-level, not the child the pixel hit-test returned:
    //   * Chromium (Opera/Chrome/Discord): web content lives in the top-level
    //     `Chrome_WidgetWin_1`; the child is a D3D layer / render widget with no
    //     usable subtree.
    //   * Steam (`SDL_app`): one SDL window draws all widgets itself; there is
    //     no child control to target, so UIA must walk the top-level's tree.
    let top_cls_now = class_name(top);
    let use_topwindow_uia = target_cls == "Intermediate D3D Window"
        || target_cls == "Chrome_RenderWidgetHostHWND"
        || top_cls_now == "Chrome_WidgetWin_1"
        || top_cls_now == "Chrome_WidgetWin_0"
        || top_cls_now == "SDL_app";

    // Detect Chromium/Electron apps. For these, PostMessage WM_LBUTTONDOWN
    // on the render widget can sometimes trigger a click in addition to UIA
    // Invoke, causing DOUBLE-ACTIVATION (e.g. a mute button in Discord
    // toggles twice → appears as "self-release").
    // Fix: for Chromium apps, skip PostMessage mouse click and rely on UIA
    // Invoke only. PostMessage is still sent for mouse-move and keyboard.
    let is_chromium = top_cls_now == "Chrome_WidgetWin_1"
        || top_cls_now == "Chrome_WidgetWin_0"
        || target_cls == "Chrome_RenderWidgetHostHWND";




    // NOTE: no per-click eprintln! here — clicking repeatedly (or dragging)
    // flooded the log sink, which writes to a file and added latency to every
    // click. Diagnostics for input resolution live in the UIA layer only.

    // PostMessage path FIRST (dependable for classic controls; harmless for
    // modern apps that ignore it). We activate the window without touching the
    // GLOBAL foreground so Minecraft never thinks it lost focus (which would
    // make it fire ESC).
    //
    // For Chromium apps, skip PostMessage for mouse clicks: PostMessage on
    // Chrome_RenderWidgetHostHWND can sometimes trigger a click in addition
    // to UIA Invoke, causing double-activation. Keep mouse-move for hover.
    unsafe {
        // Skip activate_target for Chromium entirely: WM_ACTIVATE(WA_ACTIVE)
        // causes Discord/Electron apps to call SetForegroundWindow internally,
        // which steals Minecraft's foreground and fires phantom ESC.
        if pressed && !is_chromium {
            activate_target(top, target);
        }
        if !is_chromium {
            let _ = PostMessageW(target, WM_MOUSEMOVE, move_wparam, lparam);
            let _ = PostMessageW(target, msg, wparam, lparam);
        } else if pressed && super::process::is_on_hidden_desktop(top) {
            // Hidden desktop Chromium: PostMessage to top-level for browser
            // chrome (Opera tabs, address bar).
            let _ = PostMessageW(top, WM_MOUSEMOVE, move_wparam, lparam);
            let _ = PostMessageW(top, msg, wparam, lparam);
        }
        // Visible desktop Chromium: NO PostMessage at all —
        // WM_LBUTTONDOWN to Chrome_RenderWidgetHostHWND or Chrome_WidgetWin_1
        // triggers SetForegroundWindow → phantom ESC.
    }

    // UI Automation Invoke path — ONLY for hidden desktop Chromium and
    // non-Chromium apps. For visible desktop Chromium, PostMessage alone
    // handles clicks (UIA COM calls cause focus stealing / phantom ESC).
    //
    // Modern apps (Chromium / Electron like Discord, UWP like Calculator)
    // on the HIDDEN desktop ignore synthesized PostMessage mouse input, so
    // without UIA they render but never react. Our UIA path drives the
    // element through Invoke/Select/Toggle/Expand on the app's own
    // automation tree (resolved via ElementFromHandle, not ElementFromPoint)
    // and does NOT call SetForegroundWindow, so it does not steal Minecraft's
    // global foreground. On the VISIBLE desktop, PostMessage works and UIA
    // is skipped — UIA COM calls to visible Chromium trigger
    // SetForegroundWindow internally, causing phantom ESC.
        let on_hidden = super::process::is_on_hidden_desktop(top);
        let skip_uia = is_chromium && !on_hidden;
    if pressed
        && button == 0x110
        && !is_classic_list
        && !is_text_edit
        && !skip_uia
    {
        // Choose the window whose automation tree we resolve against, and the
        // child we compute screen coordinates from.
        //
        // For a CHROMIUM surface the deepest child (`Intermediate D3D Window` or
        // `Chrome_RenderWidgetHostHWND`) has NO usable automation subtree — the
        // real, actionable web element tree hangs off the TOP-LEVEL window
        // (`Chrome_WidgetWin_1`). So we resolve UIA against `top`. The click
        // POINT must still be in true screen pixels; the D3D layer and the
        // top-level share the same client origin (the D3D layer fills the web
        // viewport), so converting the point via the `target` child is correct.
        let uia_hwnd = if use_topwindow_uia { top } else { target };


        let mut pt = POINT { x: lx, y: ly };
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(target, &mut pt);
        }
        // Pass the resolved HWND so UIA uses the window's own automation tree
        // (ElementFromHandle) rather than ElementFromPoint, which would hit-test
        // the visible desktop (Minecraft) instead.
        //
        // `on_hidden` lets the UIA fallback decide whether it may SetFocus the
        // element: that drives clicks into hidden-desktop web content (Opera,
        // hidden-desktop Chromium), but is suppressed for visible-desktop
        // windows where SetFocus would steal Minecraft's focus and fire a
        // phantom ESC. We resolve it against the TOP-LEVEL window.
        // `on_hidden` is already defined above.
        // SetFocus in the CLICK path is what makes clicking inside Opera/Chrome
        // WEB CONTENT actually work: much of a web page (a video, a canvas, a
        // custom-drawn control, an empty area of a page) exposes NO actionable
        // UIA pattern, so the only way a click "lands" there is to focus the
        // element the user pointed at. Forbidding SetFocus for Chromium is what
        // regressed clicking in Opera ("can't click anything in Opera").
        //
        // We only allow it for HIDDEN-desktop windows: Minecraft lives on the
        // VISIBLE desktop, so focusing a hidden-desktop window never pulls the
        // system foreground off Minecraft (no phantom ESC, no lost pointer grab).
        //
        // The "Opera types then stops" keyboard freeze was a separate path: the
        // TEXT-ENTRY route (append_focused_text) is the one that must not
        // SetFocus for Chromium, and it already never does — Chromium text goes
        // through WM_CHAR PostMessage (is_uwp == false), so this click-path
        // SetFocus does not reintroduce the typing freeze.
        let allow_focus = on_hidden;
        super::uia::invoke_at_screen(uia_hwnd.0 as isize, pt.x, pt.y, allow_focus);


    }
    serial

}












pub fn pointer_axis(state: &mut WindowMod, axis: i32, value: f64) {
    if state.pointer_focus_ptr == 0 {
        return;
    }
    let Some(surface) = ptr_to_ref::<super::state::WinSurface>(state.pointer_focus_ptr) else {
        return;
    };
    let Some(toplevel) = ptr_to_ref::<super::state::WinToplevel>(surface.toplevel_ptr) else {
        return;
    };

    let top = toplevel.hwnd;
    let (target, lx, ly) =
        target_child_at(top, state.pointer_x as i32, state.pointer_y as i32);

    let delta = (value * 120.0) as i32;

    // PRIMARY (modern apps): scroll the UIA element under the cursor. Works for
    // WinUI/UWP scrollable areas that ignore WM_MOUSEWHEEL. Only vertical axis.
    if axis == 0 && delta != 0 {
        let mut pt = POINT { x: lx, y: ly };
        unsafe {
            let _ = ClientToScreen(target, &mut pt);
        }
        // value>0 in this engine means wheel up; delta sign follows value.
        super::uia::scroll_at(target.0 as isize, pt.x, pt.y, delta > 0);
    }

    // FALLBACK (classic controls): deliver scroll via PostMessage.
    let wparam = WPARAM((((delta as i16) as i32 as u32) << 16) as usize);

    let lparam = pack_screen_coords(target, lx, ly);

    let msg = match axis {
        0 => WM_MOUSEWHEEL,
        1 => WM_HSCROLL,
        _ => return,
    };
    unsafe {
        let _ = PostMessageW(target, msg, wparam, lparam);
    }
}



pub fn pointer_relative_motion(state: &mut WindowMod, dx: f64, dy: f64) {

    if !state.pointer_locked {
        return;
    }
    state.pointer_x += dx;
    state.pointer_y += dy;
    if state.pointer_focus_ptr != 0 {
        send_mouse_move(state, state.pointer_focus_ptr, state.pointer_x, state.pointer_y);
    }
}

pub fn maybe_pointer_lock(state: &mut WindowMod, surface_ptr: i64) -> bool {
    state.pointer_locked = state.pointer_focus_ptr == surface_ptr;
    state.pointer_locked
}

pub fn pointer_unlock(state: &mut WindowMod) {
    state.pointer_locked = false;
}

/// Keyboard input delivered via Java's pressKey/releaseKey (keyboardInput).
///
/// This is intentionally a NO-OP: all real text entry is driven by the global
/// KeyboardHandler mixin (`onPressGlobal` -> internalKeyUpdate ->
/// `keyboard_update`), which fires for every key whether or not a Minecraft
/// Screen is open. Routing this path into keyboard_update as well would
/// double-type every character.
pub fn keyboard_key(_state: &mut WindowMod, _scancode: u32, _pressed: bool) {}



pub fn keyboard_update(state: &mut WindowMod, scancode: u32, pressed: bool) {
    if state.focus_toplevel_ptr == 0 || !state.keyboard_active {
        eprintln!(
            "[windowmod] keyboard_update IGNORED scancode={scancode} pressed={pressed} (focus_toplevel_ptr={}, keyboard_active={})",
            state.focus_toplevel_ptr, state.keyboard_active,
        );
        return;
    }
    let Some(toplevel) = ptr_to_mut::<super::state::WinToplevel>(state.focus_toplevel_ptr) else {
        eprintln!("[windowmod] keyboard_update: focus_toplevel_ptr resolves to no toplevel");
        return;
    };

    let vk = unsafe { MapVirtualKeyW(scancode, MAPVK_VSC_TO_VK) } as u16;
    if vk == 0 {
        eprintln!("[windowmod] keyboard_update: scancode={scancode} mapped to vk=0, skipping");
        return;
    }

    let top = toplevel.hwnd;

    // Resolve the window that should receive the key event.
    //
    // On a hidden desktop nothing ever SetFocus()es the deep child edit control
    // (e.g. Notepad's `RichEditD2DPT`, an edit box inside a dialog), so
    // GetGUIThreadInfo().hwnd_focus usually reports only the TOP-LEVEL window.
    // Posting WM_KEYDOWN/WM_CHAR to the top-level does nothing visible because
    // the real text control never sees them — which is exactly why typing did
    // not work even though the key events were delivered.
    //
    // The mouse path already solves this with `target_child_at`, which hit-tests
    // down the child window tree to the deepest control under the cursor. We
    // reuse it here: the last place the user pointed/clicked is where they
    // expect the caret to be. We fall back to the thread's focus window (then
    // the top-level) when we have no usable pointer position.
    let focus_win = thread_focus_window(top);
    let mut target = if state.pointer_x > 0.0 || state.pointer_y > 0.0 {
        let (child, _lx, _ly) =
            target_child_at(top, state.pointer_x as i32, state.pointer_y as i32);
        // Prefer the deepest child under the cursor; if hit-testing returned the
        // top-level itself, use the thread focus window instead.
        if child != top { child } else { focus_win }
    } else {
        focus_win
    };

    // CHROMIUM redirect: web pages receive keystrokes through the
    // `Chrome_RenderWidgetHostHWND`, NOT the `Intermediate D3D Window` GPU layer
    // that the hit test lands on. If our target is the D3D layer (or the
    // top-level itself for a Chromium window), retarget the render widget so
    // typing into Opera/Chrome address bars and web inputs actually lands.
    if class_name(target) == "Intermediate D3D Window" || target == top {
        if let Some((widget, _wx, _wy)) =
            chromium_input_target(top, state.pointer_x as i32, state.pointer_y as i32)
        {
            target = widget;
        }
    }


    let scan = (scancode & 0xFF) as u32;
    let wparam = WPARAM(vk as usize);
    let kbd_state = build_keyboard_state();

    let msg = if pressed { WM_KEYDOWN } else { WM_KEYUP };
    let repeat = if pressed { 1u32 } else { 0u32 };
    let lparam = LPARAM(
        (repeat | (scan << 16) | if !pressed { 1 << 30 | 1 << 31 } else { 0 }) as isize,
    );

    let mut chars = [0u16; 4];
    let char_count = if pressed {
        unsafe {
            let layout = GetKeyboardLayout(0);
            let count = ToUnicodeEx(
                vk as u32, scan, kbd_state.as_ptr(),
                chars.as_mut_ptr(), 4, 0, layout,
            );
            if count > 0 { count as usize } else { 0 }
        }
    } else {
        0
    };

    // Classify the TOP-LEVEL window to pick EXACTLY ONE text-entry path, so no
    // character is ever typed twice.
    let top_cls = class_name(top);

    // CHROMIUM/Electron (Opera, Chrome, Discord, VS Code). Once their
    // accessibility tree is woken, these windows ACCEPT the synthesized
    // WM_KEYDOWN + WM_CHAR we PostMessage and turn it into a typed character
    // themselves. So Chromium uses the SAME classic WM_CHAR path — and must NOT
    // also go through the UIA ValuePattern append, or every character is typed
    // TWICE ("привет" -> "ппррииввеетт"). This is the core dedup fix.
    let is_chromium = top_cls == "Chrome_WidgetWin_1" || top_cls == "Chrome_WidgetWin_0";

    // TRUE UWP/WinUI renderers and Steam's SDL UI genuinely IGNORE synthesized
    // WM_CHAR, so they need the UIA ValuePattern text path instead. Chromium is
    // deliberately EXCLUDED here — it gets the classic WM_CHAR path above.
    let is_uwp = top_cls.contains("Microsoft.UI.")
        || top_cls.contains("Windows.UI.")
        || top_cls.contains("ApplicationFrameWindow")
        // Steam's client UI is a single SDL-hosted window (`SDL_app`) that
        // renders its own widgets and IGNORES synthesized WM_CHAR.
        || top_cls == "SDL_app";

    // NOTE: no per-keystroke eprintln! here — typing floods the log sink (which
    // writes to a file) and adds latency to every key while typing fast.

    // Does this keystroke produce a PRINTABLE character (text), as opposed to a

    // pure navigation/control key (arrows, Enter, Backspace, Tab, shortcuts)?
    // ToUnicodeEx returns chars for printable keys; we treat anything < 0x20
    // (and DEL 0x7F) as non-printable control output.
    let produces_text = chars
        .iter()
        .take(char_count)
        .any(|&ch| ch >= 0x20 && ch != 0x7F);

    // PostMessage path: deliver the key event to the focused window inside the
    // target app's own thread. activate_target uses only thread-local messages
    // (no global SetForegroundWindow), so Minecraft never thinks it lost focus.
    //
    // CRITICAL — avoid DOUBLE-TYPING ("привет" -> "ппррииввеетт"):
    //
    // A character can reach an edit control by TWO routes, and sending both
    // doubles it:
    //   1. WM_KEYDOWN of a printable key, which the control's OWN message loop
    //      turns into a WM_CHAR via TranslateMessage.
    //   2. The explicit WM_CHAR we PostMessage ourselves.
    //
    // Classic Win32 edit controls (Notepad's `RichEditD2DPT`), game engines
    // (`Engine`) and Chromium ALL perform their own WM_KEYDOWN->WM_CHAR
    // translation, so when we sent WM_KEYDOWN *and* WM_CHAR for the same key the
    // character appeared twice. The log confirmed every printable key produced
    // exactly one keyboard_update yet the visible text was doubled — proof the
    // duplication is the two message routes, not two events.
    //
    // Fix: pick exactly ONE route per key:
    //   * PRINTABLE keys  -> send ONLY WM_CHAR (the character itself). Do NOT
    //     send WM_KEYDOWN/WM_KEYUP for them, so the control cannot synthesize a
    //     second WM_CHAR.
    //   * NON-PRINTABLE keys (arrows, Enter, Backspace, Tab, Esc, shortcuts,
    //     and modifier combos) -> send WM_KEYDOWN/WM_KEYUP (these carry no
    //     WM_CHAR, so there is nothing to double; they drive navigation/editing).
    //
    // UWP/WinUI/SDL windows ignore synthesized WM_CHAR entirely, so for them we
    // skip the WM_CHAR route and drive printable text through the UIA
    // ValuePattern path below (WM_KEYDOWN/WM_KEYUP for navigation still apply).
    unsafe {
        activate_target(top, target);

        // Non-printable keys (and key-up of every key) go through WM_KEYDOWN/
        // WM_KEYUP. Printable key-DOWN is delivered via WM_CHAR below instead, so
        // we do NOT also post its WM_KEYDOWN (which would translate to a 2nd char).
        let is_printable_down = pressed && produces_text;
        if !is_printable_down {
            let _ = PostMessageW(target, msg, wparam, lparam);
        }

        // Printable characters: classic Win32 + Chromium accept WM_CHAR directly.
        // This is the SINGLE text route for them (no WM_KEYDOWN above), so each
        // character is typed exactly once. UWP/SDL ignore WM_CHAR -> handled via
        // UIA below.
        if pressed && !is_uwp {
            for &ch in chars.iter().take(char_count) {
                if ch >= 0x20 || ch == 0x08 || ch == 0x09 || ch == 0x0D {
                    let _ = PostMessageW(target, WM_CHAR, WPARAM(ch as usize), lparam);
                }
            }
        }
    }


    // UWP/SDL apps get text entry EXCLUSIVELY through the UIA ValuePattern path
    // on key-DOWN for printable characters (we suppressed WM_CHAR for them above,
    // so there is exactly one text path and no double-typing). Chromium is NOT
    // included here — it already typed via WM_CHAR.
    //
    // We pass `on_hidden` down to the UIA layer so it can decide whether it is
    // allowed to call `IUIAutomationElement::SetFocus`. That call is the cause of
    // the "can't type anything after playing a video / opening a second tab"
    // freeze: on a VISIBLE-desktop window SetFocus yanks the system foreground
    // away from Minecraft, GLFW reports focus loss, and the captured keyboard can
    // no longer deliver input. On the HIDDEN desktop it is harmless (Minecraft
    // never loses its foreground), so SetFocus is gated on `on_hidden` inside the
    // UIA layer — mirroring the mouse Invoke path.
    let on_hidden = super::process::is_on_hidden_desktop(top);
    if is_uwp && pressed && char_count > 0 {
        let mut typed = String::new();
        for &ch in chars.iter().take(char_count) {
            if ch >= 0x20 {
                if let Some(c) = char::from_u32(ch as u32) {
                    typed.push(c);
                }
            }
        }
        if !typed.is_empty() {
            // UWP/SDL windows are NOT Chromium, so SetFocus is allowed when the
            // window is on the hidden desktop (it lands the caret without
            // stealing Minecraft's foreground). We still pass the Chromium guard
            // for symmetry with the mouse path.
            let allow_focus = on_hidden && !is_chromium;
            super::uia::append_focused_text(top.0 as isize, typed, allow_focus);
        }

    }
}










pub fn activate_keyboard(state: &mut WindowMod) {
    state.keyboard_active = true;
}

pub fn deactivate_keyboard(state: &mut WindowMod) {
    state.keyboard_active = false;
    state.pointer_locked = false;
}

pub fn focus_toplevel(state: &mut WindowMod, toplevel_ptr: Option<i64>) {
    let new_ptr = toplevel_ptr.unwrap_or(0);

    // Tell the capture layer which window's HWND is being VIEWED right now, so
    // that window's capture thread runs at full frame rate and every other
    // window's throttles down (the off-screen windows are invisible to the
    // player, so capturing them at 60 Hz with the expensive PrintWindow blit was
    // the real source of the lag). Resolve the toplevel's HWND from its pointer.
    let focused_hwnd = new_ptr
        .and_then_hwnd()
        .unwrap_or(0);
    super::capture::set_focused_capture_hwnd(focused_hwnd);

    if new_ptr != 0 && new_ptr == state.focus_toplevel_ptr {
        return;
    }
    state.focus_toplevel_ptr = new_ptr;
    if let Some(ptr) = toplevel_ptr {
        if let Some(surface_ptr) = state.surface_for_toplevel(ptr) {
            state.pointer_focus_ptr = surface_ptr;
        }
    }
}

/// Helper: resolve the HWND (as isize) of a toplevel pointer, or None.
trait ToplevelHwnd {
    fn and_then_hwnd(self) -> Option<isize>;
}
impl ToplevelHwnd for i64 {
    fn and_then_hwnd(self) -> Option<isize> {
        if self == 0 {
            return None;
        }
        ptr_to_ref::<super::state::WinToplevel>(self).map(|t| t.hwnd.0 as isize)
    }
}


pub fn minimize_toplevel(toplevel_ptr: i64) {
    if let Some(t) = ptr_to_ref::<super::state::WinToplevel>(toplevel_ptr) {
        unsafe {
            let _ = ShowWindow(t.hwnd, SW_MINIMIZE);
        }
    }
}

fn send_mouse_move(state: &WindowMod, surface_ptr: i64, x: f64, y: f64) {
    let Some(surface) = ptr_to_ref::<super::state::WinSurface>(surface_ptr) else {
        return;
    };
    let Some(toplevel) = ptr_to_ref::<super::state::WinToplevel>(surface.toplevel_ptr) else {
        return;
    };
    let top = toplevel.hwnd;
    let (target, lx, ly) = target_child_at(top, x as i32, y as i32);

    // While the left button is held, WM_MOUSEMOVE MUST carry MK_LBUTTON
    // (0x0001) so the control understands this is a drag and EXTENDS the text
    // selection. Without it the edit control treats every move as a hover and
    // click-and-drag selection never grows. This is what makes mouse text
    // selection work in Notepad and other edit controls.
    let wparam = if state.left_button_down {
        WPARAM(0x0001) // MK_LBUTTON
    } else {
        WPARAM(0)
    };

    // Deliver cursor move via PostMessage (dependable on the hidden desktop).
    let lparam = pack_coords_i(lx, ly);
    unsafe {
        let _ = PostMessageW(target, WM_MOUSEMOVE, wparam, lparam);
    }
}





fn pack_coords_i(x: i32, y: i32) -> LPARAM {
    let ix = x.max(0);
    let iy = y.max(0);
    LPARAM(((iy as u32) << 16 | (ix as u32 & 0xFFFF)) as isize)
}

/// Pack a point given in `target`'s client coords as SCREEN coords (for wheel).
fn pack_screen_coords(target: HWND, lx: i32, ly: i32) -> LPARAM {
    let mut pt = POINT { x: lx, y: ly };
    unsafe {
        let _ = ClientToScreen(target, &mut pt);
    }
    LPARAM(((pt.y as u32) << 16 | (pt.x as u32 & 0xFFFF)) as isize)
}

// Keep the symbol referenced so the linker/cfg stays happy if unused.
#[allow(dead_code)]
fn _keep(mc: isize) {
    unsafe {
        let _ = SetForegroundWindow(HWND(mc as *mut _));
    }
}
