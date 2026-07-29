//! Hidden-desktop input injection — DISABLED.
//!
//! The original idea here was a background thread bound to the hidden desktop
//! (via `SetThreadDesktop`) that injected *real* input with `SetForegroundWindow`
//! + `SendInput`, so modern apps (WinUI 3 / Chromium / UWP) — which ignore
//! synthesized `PostMessage` — would accept it.
//!
//! In practice this approach was unreliable and actively harmful:
//!   * `SendInput` injects into the input desktop's *focus*, not a specific
//!     window. On a hidden desktop there is no real interactive session, so the
//!     events landed on the wrong window (or nowhere), and text/clicks were
//!     lost entirely.
//!   * Calling `SetForegroundWindow` and injecting real input from the hidden
//!     desktop interfered with window-launch detection on the visible desktop,
//!     so newly launched apps stopped being found and shown.
//!   * Because `send()` returned `true` whenever the hidden desktop merely
//!     *existed*, it suppressed the proven `PostMessage` fallback in `input.rs`,
//!     which broke ALL input.
//!
//! The injector is therefore disabled: `send()` is a no-op that always returns
//! `false`, so every caller in `input.rs` uses the dependable `PostMessage`
//! path. The command enum is kept so the call sites compile unchanged; if a
//! real-input strategy is revisited later it can be reimplemented behind this
//! same API.

/// One injectable input action, expressed in the target window's CLIENT
/// coordinates. Currently unused (the injector is disabled) but retained so the
/// call sites in `input.rs` keep compiling.
#[allow(dead_code)]
pub enum InputCmd {
    /// Move the cursor to client (x,y) of `hwnd` (no buttons).
    Move { hwnd: isize, x: i32, y: i32 },
    /// Press or release a mouse button at client (x,y) of `hwnd`.
    /// `button`: 0=left, 1=right, 2=middle.
    Button { hwnd: isize, x: i32, y: i32, button: u8, down: bool },
    /// Scroll wheel by `delta` (in WHEEL_DELTA units * 120) at client (x,y).
    Wheel { hwnd: isize, x: i32, y: i32, delta: i32 },
    /// A keyboard key by scan code, optionally with a Unicode char.
    Key { hwnd: isize, scancode: u16, unicode: u16, down: bool },
}

/// Disabled: always returns `false` so callers fall back to `PostMessage`.
///
/// No hidden-desktop thread is spawned and no `SendInput`/`SetForegroundWindow`
/// is performed, so launching apps and delivering input are no longer
/// interfered with.
#[allow(unused_variables)]
pub fn send(cmd: InputCmd) -> bool {
    false
}
