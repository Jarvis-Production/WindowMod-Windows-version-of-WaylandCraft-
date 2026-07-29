//! UI Automation (UIA) input for windows living on the HIDDEN desktop.
//!
//! Modern apps (WinUI 3 / UWP / Chromium) render through DirectComposition /
//! InputSite and ignore synthesized `PostMessage` mouse/keyboard messages, and
//! `SendInput` can't be targeted at a specific window on a hidden desktop. UI
//! Automation, however, drives the app through its *automation tree* — it does
//! not depend on the window being visible, focused, or on the input desktop, so
//! it works for windows parked on our hidden desktop.
//!
//! IMPORTANT: we must NOT use `ElementFromPoint`, because that hit-tests the
//! VISIBLE desktop (where Minecraft's own window lives) and would return the
//! wrong element. Instead we resolve the target window's automation tree via
//! `ElementFromHandle(hwnd)` and walk it ourselves to find the deepest element
//! whose bounding rectangle contains the requested point — which is correct
//! regardless of the window's desktop or on-screen position.
//!
//! All UIA calls run on a DEDICATED background thread that owns an MTA COM
//! apartment, so we never touch the apartment state of Minecraft's render
//! thread. Commands are delivered over an mpsc channel.

use std::collections::HashSet;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};


use windows::core::Interface;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};



use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, ExpandCollapseState_Collapsed, IUIAutomation, IUIAutomationElement,
    IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern, IUIAutomationScrollPattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationValuePattern,
    ScrollAmount_LargeDecrement, ScrollAmount_LargeIncrement, TreeScope_Children,
    TreeScope_Descendants,

    UIA_ExpandCollapsePatternId, UIA_InvokePatternId, UIA_ScrollPatternId,
    UIA_SelectionItemPatternId, UIA_TogglePatternId, UIA_ValuePatternId,
};


/// One UIA action to perform on the background thread.
pub enum UiaCmd {
    /// Invoke the default action of the element at the given SCREEN point that
    /// belongs to window `hwnd`. We resolve the element via the window's own
    /// automation tree (not ElementFromPoint), so it works on the hidden
    /// desktop. Coordinates are absolute screen pixels.
    InvokeAt { hwnd: isize, sx: i32, sy: i32, allow_focus: bool },


    /// Append `text` to the focused value-providing element. `hwnd` is the
    /// target window: we look for the focused element inside that window's own
    /// automation tree first (correct on the hidden desktop), then fall back to
    /// the global focused element. Used for keyboard text entry into WinUI/UWP
    /// edit controls that ignore PostMessage.
    ///
    /// `allow_focus` gates `IUIAutomationElement::SetFocus`. SetFocus transfers
    /// the system INPUT FOCUS to the target window's thread. Even for a window
    /// on the HIDDEN desktop this can make GLFW report a focus change on
    /// Minecraft's window, after which the captured keyboard stops delivering
    /// input — the "Opera types for ~5 seconds then stops" freeze, which begins
    /// exactly once a click wakes the Chromium UIA tree and the fallback calls
    /// SetFocus. Chromium/Electron accept synthesized WM_CHAR/WM_KEYDOWN
    /// directly once their accessibility tree is awake, so they NEVER need
    /// SetFocus; the caller passes `allow_focus=false` for them.
    AppendFocusedText { hwnd: isize, text: String, allow_focus: bool },



    /// Scroll the element of `hwnd` at a screen point. `up` = scroll up.
    ScrollAt { hwnd: isize, sx: i32, sy: i32, up: bool },
}


static SENDER: OnceLock<Option<Sender<UiaCmd>>> = OnceLock::new();

/// Lazily spawn the UIA worker thread and return its command sender.
fn sender() -> Option<&'static Sender<UiaCmd>> {
    SENDER
        .get_or_init(|| {
            let (tx, rx) = channel::<UiaCmd>();
            std::thread::spawn(move || uia_thread_main(rx));
            Some(tx)
        })
        .as_ref()
}

/// Queue an "invoke element of `hwnd` at screen point" command.
///
/// `allow_focus` controls whether the no-actionable-pattern fallback is allowed
/// to call `IUIAutomationElement::SetFocus`. The caller must pass `false` for
/// windows where stealing the system input focus would break Minecraft's
/// captured keyboard — i.e. VISIBLE-desktop windows AND Chromium/Electron
/// windows (which accept synthesized WM_CHAR directly and never need focus).
pub fn invoke_at_screen(hwnd: isize, sx: i32, sy: i32, allow_focus: bool) -> bool {
    match sender() {
        Some(tx) => tx.send(UiaCmd::InvokeAt { hwnd, sx, sy, allow_focus }).is_ok(),
        None => false,
    }
}


/// Queue an "append text to focused element" command (keyboard text entry).
/// `hwnd` is the target window whose automation tree is searched for the
/// focused value element before falling back to the global focused element.
///
/// `allow_focus` gates the internal `SetFocus()` call so it never steals
/// Minecraft's foreground (which froze keyboard input). See the enum doc.
pub fn append_focused_text(hwnd: isize, text: String, allow_focus: bool) -> bool {
    match sender() {
        Some(tx) => tx
            .send(UiaCmd::AppendFocusedText { hwnd, text, allow_focus })
            .is_ok(),
        None => false,
    }
}




/// Queue a "scroll element of `hwnd` at screen point" command.
pub fn scroll_at(hwnd: isize, sx: i32, sy: i32, up: bool) -> bool {
    match sender() {
        Some(tx) => tx.send(UiaCmd::ScrollAt { hwnd, sx, sy, up }).is_ok(),
        None => false,
    }
}


fn uia_thread_main(rx: std::sync::mpsc::Receiver<UiaCmd>) {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if hr.is_err() {
            eprintln!("[windowmod][uia] CoInitializeEx failed: {hr:?}");
            return;
        }
    }

    let automation: IUIAutomation = match unsafe {
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
    } {
        Ok(a) => {
            eprintln!("[windowmod][uia] IUIAutomation created");
            a
        }
        Err(e) => {
            eprintln!("[windowmod][uia] CoCreateInstance(CUIAutomation) failed: {e}");
            return;
        }
    };

    while let Ok(cmd) = rx.recv() {
        // A single UIA call (ElementFromHandle + a full-subtree FindAll) can
        // BLOCK for a long time on a busy Chromium renderer — e.g. while a
        // YouTube video plays in Opera. If the user keeps clicking, those
        // commands pile up in the channel and are processed one-by-one long
        // after they are relevant, so the app appears frozen ("video makes
        // everything stop responding"). To stay responsive we DRAIN queued
        // CLICK (InvokeAt) and SCROLL commands and keep only the most recent —
        // stale clicks from seconds ago are worthless, only the latest matters.
        //
        // IMPORTANT: we must NOT coalesce AppendFocusedText (typed text). Each
        // keystroke is distinct; dropping intermediate ones would lose
        // characters ("abc" -> "c"). So text commands are always processed
        // individually, and draining stops as soon as a text command is seen.
        let mut latest = cmd;
        if !matches!(latest, UiaCmd::AppendFocusedText { .. }) {
            while let Ok(next) = rx.try_recv() {
                // Stop draining if we hit a text command — process it next loop.
                if matches!(next, UiaCmd::AppendFocusedText { .. }) {
                    // Run the click/scroll we have, then the text command will be
                    // picked up by the next recv(). We can't easily push it back,
                    // so handle the text command immediately after the match by
                    // processing `latest` first. Simplest correct behaviour:
                    // process `latest`, then `next`.
                    process_cmd(&automation, latest);
                    latest = next;
                    break;
                }
                latest = next;
            }
        }

        process_cmd(&automation, latest);
    }
}

/// Execute a single resolved UIA command.
fn process_cmd(automation: &IUIAutomation, cmd: UiaCmd) {
    match cmd {
        UiaCmd::InvokeAt { hwnd, sx, sy, allow_focus } => {
            if let Err(e) = invoke_element_at(automation, hwnd, sx, sy, allow_focus) {
                eprintln!("[windowmod][uia] invoke_at hwnd={hwnd:#x} ({sx},{sy}) failed: {e}");
            }
        }

        UiaCmd::AppendFocusedText { hwnd, text, allow_focus } => {
            if let Err(e) = append_focused_text_impl(automation, hwnd, &text, allow_focus) {
                eprintln!("[windowmod][uia] append_focused_text failed: {e}");
            }
        }



        UiaCmd::ScrollAt { hwnd, sx, sy, up } => {
            if let Err(e) = scroll_element_at(automation, hwnd, sx, sy, up) {
                eprintln!("[windowmod][uia] scroll_at hwnd={hwnd:#x} ({sx},{sy}) failed: {e}");
            }
        }
    }
}




/// Wake up a Chromium/Electron app's accessibility (UI Automation) tree.
///
/// Chromium-based apps (Opera, Chrome, Discord, Steam's CEF UI, VS Code) ship
/// their renderer accessibility tree DISABLED to save memory, and only build it
/// when an assistive-technology client asks for it. Until then,
/// `ElementFromHandle(hwnd)` returns only an EMPTY shell with no actionable web
/// elements — so Invoke/SetFocus/ValuePattern find nothing and the app looks
/// like a dead picture that ignores every click and keystroke.
///
/// The standard, documented way to trigger that build-up is to send the window
/// a `WM_GETOBJECT` with `lParam == UiaRootObjectId (-25)`. Chromium treats this
/// as "a UIA client is here" and enables its accessibility engine, after which
/// the real web/content automation tree becomes available. We send it (with a
/// short timeout so a busy renderer can never block us) before resolving the
/// element. It is harmless for non-Chromium windows — they just return their
/// normal object — and never touches the global foreground/focus.
const WM_GETOBJECT: u32 = 0x003D;
const UIA_ROOT_OBJECT_ID: isize = -25;

/// Windows we have already sent the accessibility-wake message to. The wake only
/// needs to happen ONCE per window: after the first WM_GETOBJECT(UiaRootObjectId)
/// Chromium keeps its accessibility engine enabled for that window's lifetime.
/// Sending it on every click/keystroke was the cause of the lag and the
/// "video makes everything freeze" symptom, because each send is a synchronous
/// SendMessageTimeout that stalls on a busy renderer. We cache the HWNDs here so
/// the expensive wake runs at most once.
static WOKEN_WINDOWS: OnceLock<Mutex<HashSet<isize>>> = OnceLock::new();

fn woken_windows() -> &'static Mutex<HashSet<isize>> {
    WOKEN_WINDOWS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Object id for the standard client area — Chromium also responds to this and
/// it nudges some renderers that ignore the UIA root id alone.
const OBJID_CLIENT: isize = -4;

/// Send the accessibility-wake messages to `hwnd`. Chromium/Electron build their
/// renderer accessibility tree LAZILY and ASYNCHRONOUSLY in response to a UIA
/// client probe (WM_GETOBJECT with UiaRootObjectId). We send both the UIA root
/// id and OBJID_CLIENT, with a generous timeout, because a single short probe
/// often returns before the tree is built.
///
/// IMPORTANT: unlike before, we do NOT permanently mark the window "woken" here.
/// The tree appears a few hundred ms AFTER the first probe, so if we cached
/// "woken" immediately we would never re-probe and the tree stayed empty
/// forever (the log showed every click hitting only the root Pane,
/// controlType=50033, with no children). Instead `mark_woken` is called by the
/// caller ONLY once the tree actually has content, so until then every click
/// keeps nudging the renderer to finish building it.
unsafe fn wake_accessibility(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG};

    let key = hwnd.0 as isize;
    if let Ok(set) = woken_windows().lock() {
        if set.contains(&key) {
            return; // tree already confirmed built — no need to keep probing
        }
    }

    let mut result = 0usize;
    let _ = SendMessageTimeoutW(
        hwnd,
        WM_GETOBJECT,
        WPARAM(0),
        LPARAM(UIA_ROOT_OBJECT_ID),
        SMTO_ABORTIFHUNG,
        200,
        Some(&mut result),
    );
    let _ = SendMessageTimeoutW(
        hwnd,
        WM_GETOBJECT,
        WPARAM(0),
        LPARAM(OBJID_CLIENT),
        SMTO_ABORTIFHUNG,
        200,
        Some(&mut result),
    );
}

/// Mark a window's accessibility tree as confirmed-built, so `wake_accessibility`
/// stops re-probing it. Called only after we have actually seen real content
/// (an actionable element) in the tree.
fn mark_woken(hwnd: isize) {
    if let Ok(mut set) = woken_windows().lock() {
        set.insert(hwnd);
    }
}



/// Does element's bounding rect contain the screen point (sx,sy)?
unsafe fn element_contains(element: &IUIAutomationElement, sx: i32, sy: i32) -> bool {

    if let Ok(rc) = element.CurrentBoundingRectangle() {
        // Empty rects (all zero) mean "off-screen / not laid out"; treat as no.
        if rc.right > rc.left
            && rc.bottom > rc.top
            && sx >= rc.left
            && sx < rc.right
            && sy >= rc.top
            && sy < rc.bottom
        {
            return true;
        }
    }
    false
}

/// Walk down the automation subtree of `element`, descending into the child
/// whose bounding rect contains (sx,sy), to find the deepest matching element.
/// Returns the deepest element that contains the point (or `element` itself).
unsafe fn deepest_at(
    automation: &IUIAutomation,
    element: IUIAutomationElement,
    sx: i32,
    sy: i32,
) -> IUIAutomationElement {
    let mut current = element;
    // Bound depth so a pathological tree can't loop forever.
    for _ in 0..40 {
        let Ok(cond) = automation.CreateTrueCondition() else { break };
        let Ok(children) = current.FindAll(TreeScope_Children, &cond) else { break };
        let count = children.Length().unwrap_or(0);
        let mut descended = false;
        // Iterate children in reverse so topmost (last-added) wins on overlap.
        for i in (0..count).rev() {
            let Ok(child) = children.GetElement(i) else { continue };
            if element_contains(&child, sx, sy) {
                current = child;
                descended = true;
                break;
            }
        }
        if !descended {
            break;
        }
    }
    current
}

/// Resolve the element of `hwnd` at a screen point via the window's own
/// automation tree (NOT ElementFromPoint), then invoke its default action.
fn invoke_element_at(
    automation: &IUIAutomation,
    hwnd: isize,
    sx: i32,
    sy: i32,
    allow_focus: bool,
) -> windows::core::Result<()> {


    unsafe {
        // Wake Chromium/Electron accessibility so its content tree exists before
        // we resolve elements (Opera/Discord/Steam are dead pictures otherwise).
        wake_accessibility(HWND(hwnd as *mut _));
        // Root element of the target window — works on the hidden desktop.
        let root: IUIAutomationElement =
            automation.ElementFromHandle(HWND(hwnd as *mut _))?;
        let element = deepest_at(automation, root.clone(), sx, sy);


        let name = element.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let ctrl_type = element.CurrentControlType().map(|t| t.0).unwrap_or(0);
        eprintln!(
            "[windowmod][uia] hwnd={hwnd:#x} elem at ({sx},{sy}) name='{}' controlType={}",
            name, ctrl_type,
        );

        // ExpandCollapsePattern — top-level menu bar items (File/View/...),
        // combo boxes, tree items. These must EXPAND on click, not Invoke
        // (Invoke on a menu-bar item often does nothing). We try this FIRST so
        // clicking "File" actually drops the menu down. Only expand when the
        // element is currently collapsed; otherwise fall through to Invoke so a
        // second click (or clicking an already-open menu) still works.
        if let Ok(unknown) = element.GetCurrentPattern(UIA_ExpandCollapsePatternId) {
            if let Ok(ec) = unknown.cast::<IUIAutomationExpandCollapsePattern>() {
                let collapsed = ec
                    .CurrentExpandCollapseState()
                    .map(|s| s == ExpandCollapseState_Collapsed)
                    .unwrap_or(false);
                if collapsed {
                    ec.Expand()?;
                    eprintln!("[windowmod][uia]   -> Expand()");
                    return Ok(());
                }
                // Already expanded/leaf — let Invoke below handle activation.
            }
        }

        // InvokePattern — buttons, links, menu items.
        if let Ok(unknown) = element.GetCurrentPattern(UIA_InvokePatternId) {
            if let Ok(invoke) = unknown.cast::<IUIAutomationInvokePattern>() {
                invoke.Invoke()?;
                eprintln!("[windowmod][uia]   -> Invoke()");
                return Ok(());
            }
        }

        // TogglePattern — checkboxes / toggle switches.
        if let Ok(unknown) = element.GetCurrentPattern(UIA_TogglePatternId) {
            if let Ok(toggle) = unknown.cast::<IUIAutomationTogglePattern>() {
                toggle.Toggle()?;
                eprintln!("[windowmod][uia]   -> Toggle()");
                return Ok(());
            }
        }
        // SelectionItemPattern — list items, tabs, radio buttons.
        if let Ok(unknown) = element.GetCurrentPattern(UIA_SelectionItemPatternId) {
            if let Ok(sel) = unknown.cast::<IUIAutomationSelectionItemPattern>() {
                sel.Select()?;
                eprintln!("[windowmod][uia]   -> Select()");
                return Ok(());
            }
        }

        // The element we landed on has no actionable pattern (e.g. a WinUI
        // `PopupHost`/menu container, or a Chromium root `Pane` whose web
        // content is all DESCENDANTS). Search the whole subtree of the TARGET
        // ROOT (not just `element`) for the deepest descendant at the point that
        // DOES expose an actionable pattern, and invoke it.
        //
        // We search from `root`, not `element`: the log showed `deepest_at`
        // bottoming out at the root Chromium Pane (controlType=50033) with no
        // navigable children, so searching only its subtree found nothing.
        // Searching the whole window subtree via TreeScope_Descendants reaches
        // the real web buttons/links/inputs once accessibility has built them.
        if let Some(()) = invoke_actionable_descendant(automation, &root, sx, sy) {
            // The tree had real, actionable content → it is built. Stop probing.
            mark_woken(hwnd);
            return Ok(());
        }

        // Nothing actionable at this point — fall back to focusing the element
        // so a click in a Chromium/Electron render surface (VS Code, Discord,
        // Opera's video/web content) still lands input there.
        //
        // CRITICAL nuance — only do this when the window is on the HIDDEN
        // desktop. `IUIAutomationElement::SetFocus` transfers the system INPUT
        // FOCUS to the target window. On the HIDDEN desktop that is harmless:
        // Minecraft lives on the VISIBLE desktop and never loses its focus, so
        // the click reaches the web content (this is what makes clicking inside
        // a hidden-desktop browser work). But on the VISIBLE desktop (where
        // some apps land, only parked off-screen) the very same call yanks the
        // foreground away from Minecraft, GLFW reports a focus-loss, and
        // Minecraft closes its open GUI screen — which the user sees as a
        // phantom "ESC" on every click inside VS Code. The same focus-steal also
        // breaks Chromium keyboard capture (the "Opera types ~5s then stops"
        // freeze), so the caller passes `allow_focus=false` for Chromium too.
        if allow_focus {
            let _ = element.SetFocus();
            eprintln!("[windowmod][uia]   -> no actionable pattern, SetFocus() (focus allowed)");
        } else {
            eprintln!("[windowmod][uia]   -> no actionable pattern (SetFocus skipped: avoids stealing focus)");
        }

        Ok(())


    }
}

/// Search ALL descendants of `container` for the element at screen point
/// (sx,sy) that exposes an actionable pattern (Invoke / SelectionItem / Toggle /
/// ExpandCollapse) and activate it. Returns Some(()) if something was invoked.
///
/// This is the key to clicking WinUI menu/flyout items: the element resolved by
/// the spatial walk is often a non-actionable host (`PopupHost`), while the
/// actual `MenuFlyoutItem` is a deeper descendant. We enumerate the subtree,
/// keep candidates whose bounding rect contains the point, and prefer the
/// smallest (deepest/most specific) one.
unsafe fn invoke_actionable_descendant(
    automation: &IUIAutomation,
    container: &IUIAutomationElement,
    sx: i32,
    sy: i32,
) -> Option<()> {
    let cond = automation.CreateTrueCondition().ok()?;

    // PERF: we must NOT do `FindAll(TreeScope_Descendants)` on the whole window.
    // A Chromium web page's automation tree has THOUSANDS of nodes; enumerating
    // and pattern-probing all of them takes hundreds of ms and blocks the UIA
    // worker thread, so rapid clicks in Opera pile up and get dropped ("can't
    // click anything in Opera"). Instead we do a BOUNDED, SPATIALLY-PRUNED BFS:
    // we only descend into children whose bounding rect CONTAINS the click
    // point, and we cap the total number of nodes we look at. That visits only
    // the handful of elements actually under the cursor.
    let mut best: Option<(IUIAutomationElement, i64)> = None;
    let mut queue: Vec<IUIAutomationElement> = vec![container.clone()];
    let mut visited = 0usize;
    const MAX_VISITED: usize = 400;

    while let Some(node) = queue.pop() {
        visited += 1;
        if visited > MAX_VISITED {
            break;
        }

        // Only consider nodes that actually contain the point.
        if element_contains(&node, sx, sy) {
            // Must expose at least one actionable pattern.
            let actionable = node.GetCurrentPattern(UIA_InvokePatternId).is_ok()
                || node.GetCurrentPattern(UIA_SelectionItemPatternId).is_ok()
                || node.GetCurrentPattern(UIA_TogglePatternId).is_ok()
                || node.GetCurrentPattern(UIA_ExpandCollapsePatternId).is_ok();
            if actionable {
                // Prefer the smallest-area candidate (deepest, most specific).
                let area = if let Ok(rc) = node.CurrentBoundingRectangle() {
                    ((rc.right - rc.left) as i64) * ((rc.bottom - rc.top) as i64)
                } else {
                    i64::MAX
                };
                match &best {
                    Some((_, best_area)) if *best_area <= area => {}
                    _ => best = Some((node.clone(), area)),
                }
            }
        }

        // Descend ONLY into children that contain the point — this prunes the
        // vast majority of the tree so the walk stays cheap.
        if let Ok(children) = node.FindAll(TreeScope_Children, &cond) {
            let count = children.Length().unwrap_or(0);
            for i in 0..count {
                if let Ok(child) = children.GetElement(i) {
                    if element_contains(&child, sx, sy) {
                        queue.push(child);
                    }
                }
            }
        }
    }

    let (el, _) = best?;

    let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();

    if let Ok(unknown) = el.GetCurrentPattern(UIA_InvokePatternId) {
        if let Ok(invoke) = unknown.cast::<IUIAutomationInvokePattern>() {
            if invoke.Invoke().is_ok() {
                eprintln!("[windowmod][uia]   -> descendant Invoke() '{}'", name);
                return Some(());
            }
        }
    }
    if let Ok(unknown) = el.GetCurrentPattern(UIA_SelectionItemPatternId) {
        if let Ok(sel) = unknown.cast::<IUIAutomationSelectionItemPattern>() {
            if sel.Select().is_ok() {
                eprintln!("[windowmod][uia]   -> descendant Select() '{}'", name);
                return Some(());
            }
        }
    }
    if let Ok(unknown) = el.GetCurrentPattern(UIA_TogglePatternId) {
        if let Ok(toggle) = unknown.cast::<IUIAutomationTogglePattern>() {
            if toggle.Toggle().is_ok() {
                eprintln!("[windowmod][uia]   -> descendant Toggle() '{}'", name);
                return Some(());
            }
        }
    }
    if let Ok(unknown) = el.GetCurrentPattern(UIA_ExpandCollapsePatternId) {
        if let Ok(ec) = unknown.cast::<IUIAutomationExpandCollapsePattern>() {
            if ec.Expand().is_ok() {
                eprintln!("[windowmod][uia]   -> descendant Expand() '{}'", name);
                return Some(());
            }
        }
    }
    None
}


/// Append `text` to the currently focused element via ValuePattern.
///
/// UIA ValuePattern can only *replace* the whole value, so we read the current
/// value and write back `current + text`. This works for simple edit controls
/// (search boxes, address bars). Rich document editors (Notepad's editor) often
/// do NOT expose ValuePattern; in that case this fails gracefully and the
/// caller's PostMessage fallback (WM_CHAR) is the only option.
fn append_focused_text_impl(
    automation: &IUIAutomation,
    hwnd: isize,
    text: &str,
    allow_focus: bool,
) -> windows::core::Result<()> {

    unsafe {
        // Prefer the focused element inside the TARGET window's own automation
        // tree. On the hidden desktop the GLOBAL focused element (the one
        // GetFocusedElement returns) is usually Minecraft, not our offscreen
        // app, so resolving via the window handle is what makes typing land in
        // the right control. Fall back to the global focused element only if
        // the window's tree has no recorded focus.
        let element: IUIAutomationElement = focused_value_element(automation, hwnd)
            .or_else(|| automation.GetFocusedElement().ok())
            .ok_or_else(|| {
                windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "no focused element",
                )
            })?;

        let name = element.CurrentName().map(|b| b.to_string()).unwrap_or_default();

        // Ensure the element actually has keyboard focus before writing. On the
        // HIDDEN desktop nothing ever focuses the edit control, so SetValue can
        // silently no-op; SetFocus there is safe because Minecraft lives on the
        // VISIBLE desktop and never loses its foreground.
        //
        // On the VISIBLE desktop we MUST NOT call SetFocus: it yanks the system
        // foreground away from Minecraft, GLFW reports focus loss and the
        // captured keyboard stops delivering input — the exact "can't type after
        // playing a video / opening a second tab" freeze. So we gate it on
        // `allow_focus`, mirroring the mouse Invoke path.
        if allow_focus {
            let _ = element.SetFocus();
        }



        let unknown = element.GetCurrentPattern(UIA_ValuePatternId)?;
        let value = unknown.cast::<IUIAutomationValuePattern>()?;

        // Read current value (may be empty) and append.
        let current = value.CurrentValue().map(|b| b.to_string()).unwrap_or_default();
        let mut combined = current;
        combined.push_str(text);
        let bstr = windows::core::BSTR::from(combined.as_str());
        value.SetValue(&bstr)?;
        eprintln!("[windowmod][uia] appended text to focused '{}'", name);
        Ok(())
    }
}

/// Find a value-providing element that has keyboard focus within `hwnd`'s own
/// automation tree. Walks the subtree looking for the element whose
/// `CurrentHasKeyboardFocus` is true and which exposes a ValuePattern. Returns
/// None if no such element is found (caller falls back to global focus).
fn focused_value_element(
    automation: &IUIAutomation,
    hwnd: isize,
) -> Option<IUIAutomationElement> {
    unsafe {
        // Ensure Chromium/Electron accessibility is built so the focused web
        // edit element (address bar, search box, Discord message box) is found.
        wake_accessibility(HWND(hwnd as *mut _));
        let root = automation.ElementFromHandle(HWND(hwnd as *mut _)).ok()?;
        // BFS over the window's automation subtree (bounded) for the focused

        // element. We keep it shallow/bounded so this stays cheap per keystroke.
        let cond = automation.CreateTrueCondition().ok()?;
        let mut queue = vec![root];
        let mut visited = 0usize;
        // If we find a focused element that lacks ValuePattern itself (common in
        // Chromium/Electron, where the focused node is a container and the real
        // editable field is a descendant), remember it so we can search its
        // subtree for a ValuePattern child as a second pass.
        let mut focused_without_value: Option<IUIAutomationElement> = None;
        while let Some(node) = queue.pop() {
            visited += 1;
            if visited > 400 {
                break;
            }
            let has_focus = node
                .CurrentHasKeyboardFocus()
                .map(|b| b.as_bool())
                .unwrap_or(false);
            if has_focus {
                if node.GetCurrentPattern(UIA_ValuePatternId).is_ok() {
                    return Some(node);
                }
                // Focused but no ValuePattern — keep it for the descendant search.
                if focused_without_value.is_none() {
                    focused_without_value = Some(node.clone());
                }
            }
            if let Ok(children) = node.FindAll(TreeScope_Children, &cond) {
                let count = children.Length().unwrap_or(0);
                for i in 0..count {
                    if let Ok(child) = children.GetElement(i) {
                        queue.push(child);
                    }
                }
            }
        }

        // Second pass: the focused element had no ValuePattern. Search its own
        // descendants for the FIRST element exposing a ValuePattern — this is
        // the real editable field inside a focused container (Chromium search
        // boxes, Discord's message input host).
        if let Some(focused) = focused_without_value {
            if let Ok(all) = focused.FindAll(TreeScope_Descendants, &cond) {
                let count = all.Length().unwrap_or(0);
                for i in 0..count {
                    if let Ok(el) = all.GetElement(i) {
                        if el.GetCurrentPattern(UIA_ValuePatternId).is_ok() {
                            return Some(el);
                        }
                    }
                }
            }
        }
        None
    }
}


/// Scroll the element under a screen point of `hwnd` via ScrollPattern.
fn scroll_element_at(
    automation: &IUIAutomation,
    hwnd: isize,
    sx: i32,
    sy: i32,
    up: bool,
) -> windows::core::Result<()> {
    unsafe {
        let root: IUIAutomationElement =
            automation.ElementFromHandle(HWND(hwnd as *mut _))?;
        let mut element = deepest_at(automation, root, sx, sy);

        // Walk UP the parent chain until we find an element with ScrollPattern,
        // since the deepest element (a text run) usually isn't the scroller.
        let walker = automation.ControlViewWalker()?;
        for _ in 0..12 {
            if let Ok(unknown) = element.GetCurrentPattern(UIA_ScrollPatternId) {
                if let Ok(scroll) = unknown.cast::<IUIAutomationScrollPattern>() {
                    let amount = if up {
                        ScrollAmount_LargeDecrement
                    } else {
                        ScrollAmount_LargeIncrement
                    };
                    // Horizontal unchanged (NoAmount = -1 not available; use 0
                    // semantics via same-Decrement is wrong, so pass the same
                    // vertical and let UIA ignore horizontal where unsupported).
                    scroll.Scroll(
                        windows::Win32::UI::Accessibility::ScrollAmount_NoAmount,
                        amount,
                    )?;
                    eprintln!("[windowmod][uia] scrolled {} at ({sx},{sy})", if up {"up"} else {"down"});
                    return Ok(());
                }
            }
            // Move to parent in the control view.
            match walker.GetParentElement(&element) {
                Ok(parent) => element = parent,
                Err(_) => break,
            }
        }
        eprintln!("[windowmod][uia] no ScrollPattern found at ({sx},{sy})");
        Ok(())
    }
}
