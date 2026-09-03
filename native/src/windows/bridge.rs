#![allow(non_snake_case)]
#![allow(clippy::too_many_arguments)]

use crate::java_types::*;
use crate::windows::apps::{self, RawDesktopEntry};
use crate::windows::input;
use crate::windows::process;
use crate::windows::state::{self, ptr_to_mut, ptr_to_ref, WindowMod};
use jni::objects::{JIntArray, JLongArray, JObjectArray, JPrimitiveArray, JString};
use jni::{
    bind_java_type, objects::JClass, sys::*, Env,
};
use std::path::PathBuf;
use thiserror::Error;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, SWP_NOZORDER, HWND_TOP,
};

bind_java_type! {
    rust_type = WaylandCraftBridge,
    java_type = dev.evvie.waylandcraft.bridge.WaylandCraftBridge,

    type_map {
        WLCSurface => dev.evvie.waylandcraft.bridge.WLCSurface,
        JRawDesktopEntry => dev.evvie.waylandcraft.desktop.RawDesktopEntry,
    },

    methods {
        fn get_or_create_surface(jlong) -> WLCSurface,
    },

    native_methods {
        static extern fn init {
            sig = (glfw_get_proc_address: jlong, egl_display: jlong) -> jlong,
            fn = init,
        },
        static extern fn set_win32_hwnd {
            sig = (instance: jlong, hwnd: jlong),
            fn = set_win32_hwnd,
        },
        static extern fn shutdown {
            sig = (instance: jlong),
            fn = shutdown,
        },
        static extern fn update {
            sig = (instance: jlong),
            fn = update,
        },
        static extern fn socket {
            sig = (instance: jlong) -> JString,
            fn = socket,
        },
        static extern fn x11_display {
            sig = (instance: jlong) -> JString,
            fn = x11_display,
        },
        static extern fn send_frame {
            sig = (surface_handle: jlong),
            fn = send_frame,
        },
        static extern fn update_surface_data {
            sig = (instance: jlong, surface: WLCSurface),
            fn = update_surface_data,
        },
        static extern fn toplevels {
            sig = (instance: jlong) -> jlong[],
            fn = toplevels,
        },
        static extern fn toplevel_surface {
            sig = (instance: jlong, toplevel_handle: jlong) -> jlong,
            fn = toplevel_surface,
        },
        static extern fn toplevel_title {
            sig = (toplevel_handle: jlong) -> JString,
            fn = toplevel_title,
        },
        static extern fn toplevel_app_id {
            sig = (toplevel_handle: jlong) -> JString,
            name = "toplevelAppID",
            fn = toplevel_app_id,
        },
        static extern fn toplevel_resize {
            sig = (
                toplevel_handle: jlong,
                width: jint,
                height: jint,
                interactive: jboolean
            ),
            fn = toplevel_resize,
        },
        static extern fn toplevel_resize_ovr {
            sig = (toplevel_handle: jlong, width: jint, height: jint),
            fn = toplevel_resize_ovr,
        },
        static extern fn minimize_req {
            sig = (instance: jlong) -> jlong[],
            fn = minimize_req,
        },
        static extern fn maximize_req {
            sig = (instance: jlong) -> jlong[],
            fn = maximize_req,
        },
        static extern fn unmaximize_req {
            sig = (instance: jlong) -> jlong[],
            fn = unmaximize_req,
        },
        static extern fn fullscreen_req {
            sig = (instance: jlong) -> jlong[],
            fn = fullscreen_req,
        },
        static extern fn unfullscreen_req {
            sig = (instance: jlong) -> jlong[],
            fn = unfullscreen_req,
        },
        static extern fn move_request {
            sig = (instance: jlong) -> jint[],
            fn = move_request,
        },
        static extern fn resize_request {
            sig = (instance: jlong) -> jint[],
            fn = resize_request,
        },
        static extern fn fullscreened {
            sig = (instance: jlong) -> jlong[],
            fn = fullscreened,
        },
        static extern fn toplevel_maximize {
            sig = (instance: jlong, toplevel_handle: jlong),
            fn = toplevel_maximize,
        },
        static extern fn toplevel_fullscreen {
            sig = (instance: jlong, toplevel_handle: jlong),
            fn = toplevel_fullscreen,
        },
        static extern fn kill_toplevel {
            sig = (instance: jlong, toplevel_handle: jlong) -> jboolean,
            fn = kill_toplevel,
        },

        static extern fn popups {
            sig = (instance: jlong) -> jlong[],
            fn = popups,
        },
        static extern fn popup_surface {
            sig = (instance: jlong, popup_handle: jlong) -> jlong,
            fn = popup_surface,
        },
        static extern fn popup_parent {
            sig = (instance: jlong, popup_handle: jlong) -> jlong,
            fn = popup_parent,
        },
        static extern fn popup_offset {
            sig = (popup_handle: jlong) -> jint[],
            fn = popup_offset,
        },
        static extern fn surface_xdg_geometry {
            sig = (surface_handle: jlong) -> jint[],
            name = "surfaceXDGGeometry",
            fn = surface_xdg_geometry,
        },
        static extern fn dmabufs {
            sig = (instance: jlong) -> jlong[],
            fn = dmabufs
        },
        extern fn update_surface_tree {
            sig = (instance: jlong, surface: WLCSurface) -> WLCSurface,
            fn = update_surface_tree,
        },
        static extern fn check_input_region {
            sig = (surface_handle: jlong, x: jdouble, y: jdouble) -> jboolean,
            fn = check_input_region,
        },
        static extern fn pointer_motion {
            sig = (instance: jlong, x: jdouble, y: jdouble),
            fn = pointer_motion,
        },
        static extern fn pointer_motion_focus {
            sig = (
                instance: jlong,
                surface_handle: jlong,
                x: jdouble,
                y: jdouble
            ),
            fn = pointer_motion_focus,
        },
        static extern fn pointer_rel_motion {
            sig = (instance: jlong, dx: jdouble, dy: jdouble),
            fn = pointer_rel_motion,
        },
        static extern fn maybe_pointer_lock {
            sig = (instance: jlong, surface_handle: jlong) -> jboolean,
            fn = maybe_pointer_lock,
        },
        static extern fn pointer_unlock {
            sig = (instance: jlong),
            fn = pointer_unlock,
        },
        static extern fn pointer_leave {
            sig = (instance: jlong),
            fn = pointer_leave,
        },
        static extern fn pointer_button {
            sig = (instance: jlong, button: jint, state: jint) -> jint,
            fn = pointer_button,
        },
        static extern fn pointer_axis {
            sig = (instance: jlong, axis: jint, value: jdouble),
            fn = pointer_axis,
        },
        static extern fn cursor_shape {
            sig = (instance: jlong) -> jint,
            fn = cursor_shape,
        },
        static extern fn keyboard_focus {
            sig = (instance: jlong, surface_handle: jlong),
            fn = keyboard_focus,
        },
        static extern fn keyboard_activate {
            sig = (instance: jlong),
            fn = keyboard_activate,
        },
        static extern fn keyboard_deactivate {
            sig = (instance: jlong),
            fn = keyboard_deactivate,
        },
        static extern fn keyboard_input {
            sig = (instance: jlong, scancode: jint, action: jint),
            fn = keyboard_input,
        },
        static extern fn keyboard_update {
            sig = (instance: jlong, scancode: jint, pressed: jboolean),
            fn = keyboard_update,
        },
        static extern fn output_size {
            sig = (instance: jlong) -> jint[],
            fn = output_size,
        },
        static extern fn output_bounds {
            sig = (instance: jlong) -> jint[],
            fn = output_bounds,
        },
        static extern fn output_resize {
            sig = (instance: jlong, width: jint, height: jint),
            fn = output_resize,
        },
        static extern fn output_set_bounds {
            sig = (instance: jlong, width: jint, height: jint),
            fn = output_set_bounds,
        },
        static extern fn free_surface {
            sig = (instance: jlong, surface_handle: jlong),
            fn = free_surface,
        },
        static extern fn free_toplevel {
            sig = (instance: jlong, toplevel_handle: jlong),
            fn = free_toplevel,
        },
        static extern fn free_popup {
            sig = (instance: jlong, popup_handle: jlong),
            fn = free_popup,
        },
        static extern fn load_desktop_entry {
            sig = (instance: jlong, path: JString) -> JRawDesktopEntry,
            fn = load_desktop_entry,
        },
        static extern fn load_desktop_entries {
            sig = (instance: jlong) -> JRawDesktopEntry[],
            fn = load_desktop_entries,
        },
        static extern fn render_svg {
            sig = (
                path: JString,
                width: jint,
                height: jint,
                buffer_ptr: jlong
            ) -> jboolean,
            name = "renderSVG",
            fn = render_svg,
        },
        static extern fn render_image {
            sig = (
                path: JString,
                width: jint,
                height: jint,
                buffer_ptr: jlong
            ) -> jboolean,
            name = "renderImage",
            fn = render_image,
        },
        static extern fn exec_app {
            sig = (instance: jlong, app_id: JString) -> jboolean,
            fn = exec_app,
        },
        static extern fn launch_exe {
            sig = (instance: jlong, path: JString) -> jboolean,
            fn = launch_exe,
        },
        static extern fn set_preferred_terminal {
            sig = (instance: jlong, cmd: JString),
            fn = set_preferred_terminal,
        },
        static extern fn set_keymap_default {
            sig = (instance: jlong),
            fn = set_keymap_default,
        },
        static extern fn export_keymap {
            sig = (instance: jlong) -> JString,
            fn = export_keymap,
        },
        static extern fn set_keymap_from_str {
            sig = (instance: jlong, keymap: JString) -> jboolean,
            fn = set_keymap_from_str,
        },
        static extern fn check_dnd_request {
            sig = (instance: jlong) -> jint[],
            fn = check_dnd_request,
        },
        static extern fn check_dnd_active {
            sig = (instance: jlong) -> jboolean,
            fn = check_dnd_active,
        },
        static extern fn dnd_cancel {
            sig = (instance: jlong),
            fn = dnd_cancel,
        },
        static extern fn dnd_drop {
            sig = (instance: jlong),
            fn = dnd_drop,
        },
        static extern fn dnd_motion {
            sig = (
                instance: jlong,
                surface_handle: jlong,
                x: jdouble,
                y: jdouble
            ),
            fn = dnd_motion,
        },
        static extern fn dnd_icon {
            sig = (instance: jlong) -> jlong,
            fn = dnd_icon,
        },
    },
}

#[derive(Debug, Error)]
enum BridgeError {
    #[error(transparent)]
    JniError(#[from] jni::errors::Error),
    #[error("Null instance")]
    NullInstance,
    #[error("Null toplevel")]
    NullToplevel,
    #[error("Panic during init: {0}")]
    InitPanic(String),
}

macro_rules! instance {
    ($ptr:expr) => {
        ptr_to_mut::<WindowMod>($ptr).ok_or(BridgeError::NullInstance)?
    };
}

fn init<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _glfw_get_proc_address: jlong,
    _egl_display: jlong,
) -> Result<jlong, BridgeError> {
    // Redirect native stderr (all `[windowmod]` eprintln! output) to
    // run/windowmod_native.log so diagnostics survive even when Minecraft is
    // launched without a console attached.
    super::logsink::init_once();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(WindowMod::new)) {

        Ok(instance) => {
            eprintln!("[windowmod] Initialized successfully");
            Ok(Box::into_raw(Box::new(instance)) as jlong)
        }
        Err(panic_info) => {
            let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                format!("{panic_info:?}")
            };
            eprintln!("[windowmod] PANIC: {msg}");
            Err(BridgeError::InitPanic(msg))
        }
    }
}

fn shutdown<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<(), BridgeError> {
    if instance != 0 {
        // Reclaim ownership of the state. Before dropping it, terminate every
        // process the mod launched from inside Minecraft (and their descendant
        // trees) so apps the user opened through the window mod don't keep
        // running after the game exits. Processes that were already running
        // before the mod launched them are never recorded, so they survive.
        let state = unsafe { Box::from_raw(instance as *mut WindowMod) };
        process::kill_launched_processes(&state);
        drop(state);
    }
    Ok(())
}


fn set_win32_hwnd<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
    hwnd: jlong,
) -> Result<(), BridgeError> {
    super::input::set_win32_hwnd(hwnd as isize);
    eprintln!("[windowmod] Win32 HWND set to 0x{:x}", hwnd);
    Ok(())
}

fn update<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<(), BridgeError> {
    let state = instance!(instance);
    let pending_before = state.pending_launches.len();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.update();
    }));
    if let Err(e) = result {
        let msg = if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            format!("{:?}", e)
        };
        eprintln!("[windowmod] PANIC in update(): {msg}");
    }
    let _ = pending_before;
    Ok(())
}


fn socket<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<JString<'local>, BridgeError> {
    Ok(JString::new(env, "windowmod")?)
}

fn x11_display<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<JString<'local>, BridgeError> {
    Ok(JString::null())
}

fn send_frame<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _surface_handle: jlong,
) -> Result<(), BridgeError> {
    Ok(())
}

fn toplevels<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    let state = instance!(instance);
    state::retain_toplevels(state);
    // Popups are NOT returned here: they are rendered INSIDE their owner window
    // as child surfaces (see update_surface_tree), so Java must not treat them
    // as standalone toplevel windows.
    let handles: Vec<jlong> = state
        .toplevels
        .iter()
        .filter(|t| !t.is_popup)
        .map(|t| state::ptr_of_ref(&**t))
        .collect();

    // NOTE: no eprintln! here. This runs EVERY frame; logging it (the log sink
    // writes to a file) added measurable per-frame cost and flooded the log.
    let array = JLongArray::new(env, handles.len())?;

    array.set_region(env, 0, &handles)?;
    Ok(array)
}

fn popups<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    Ok(JLongArray::new(env, 0)?)
}

fn minimize_req<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    let state = instance!(instance);
    let mut handles = Vec::new();
    for toplevel in state.toplevels.iter_mut() {
        if toplevel.requests.minimize {
            toplevel.requests.minimize = false;
            handles.push(state::ptr_of_ref(&**toplevel));
        }
    }
    for ptr in &handles {
        input::minimize_toplevel(*ptr);
    }
    let array = JLongArray::new(env, handles.len())?;
    array.set_region(env, 0, &handles)?;
    Ok(array)
}

fn collect_and_clear_req(
    state: &mut WindowMod,
    field: fn(&mut state::WinToplevel) -> &mut bool,
) -> Vec<jlong> {
    state
        .toplevels
        .iter_mut()
        .filter_map(|t| {
            let req = field(t);
            if *req {
                *req = false;
                Some(state::ptr_of_ref(&**t))
            } else {
                None
            }
        })
        .collect()
}

fn maximize_req<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    let state = instance!(instance);
    let handles = collect_and_clear_req(state, |t| &mut t.requests.maximize);
    let array = JLongArray::new(env, handles.len())?;
    array.set_region(env, 0, &handles)?;
    Ok(array)
}

fn unmaximize_req<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    let state = instance!(instance);
    let handles = collect_and_clear_req(state, |t| &mut t.requests.unmaximize);
    let array = JLongArray::new(env, handles.len())?;
    array.set_region(env, 0, &handles)?;
    Ok(array)
}

fn fullscreen_req<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    let state = instance!(instance);
    let handles = collect_and_clear_req(state, |t| &mut t.requests.fullscreen);
    let array = JLongArray::new(env, handles.len())?;
    array.set_region(env, 0, &handles)?;
    Ok(array)
}

fn unfullscreen_req<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    let state = instance!(instance);
    let handles = collect_and_clear_req(state, |t| &mut t.requests.unfullscreen);
    let array = JLongArray::new(env, handles.len())?;
    array.set_region(env, 0, &handles)?;
    Ok(array)
}

fn move_request<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jint>, BridgeError> {
    let state = instance!(instance);
    let Some(serial) = state.move_serial.take() else {
        return Ok(JPrimitiveArray::null());
    };
    let array = JIntArray::new(env, 1)?;
    array.set_region(env, 0, &[serial as jint])?;
    Ok(array)
}

fn resize_request<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jint>, BridgeError> {
    let state = instance!(instance);
    let Some((serial, edges)) = state.resize_serial.take() else {
        return Ok(JPrimitiveArray::null());
    };
    let array = JIntArray::new(env, 2)?;
    array.set_region(env, 0, &[serial as jint, edges as jint])?;
    Ok(array)
}

fn fullscreened<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    let state = instance!(instance);
    let handles: Vec<jlong> = state
        .toplevels
        .iter()
        .filter(|t| t.fullscreen)
        .map(|t| state::ptr_of_ref(&**t))
        .collect();
    let array = JLongArray::new(env, handles.len())?;
    array.set_region(env, 0, &handles)?;
    Ok(array)
}

fn toplevel_surface<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    toplevel_handle: jlong,
) -> Result<jlong, BridgeError> {
    let state = instance!(instance);
    let result = state.surface_for_toplevel(toplevel_handle).unwrap_or(0);
    // NOTE: no per-call eprintln! here — this is called for every toplevel every
    // frame, so logging it flooded the log and added per-frame cost. The "no
    // surface" case is transient (surface registered a frame later) and not
    // worth logging on the hot path.
    Ok(result)

}

fn popup_surface<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
    _popup_handle: jlong,
) -> Result<jlong, BridgeError> {
    Ok(0)
}

fn popup_parent<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
    _popup_handle: jlong,
) -> Result<jlong, BridgeError> {
    Ok(0)
}

fn popup_offset<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _popup_handle: jlong,
) -> Result<JPrimitiveArray<'local, jint>, BridgeError> {
    Ok(JIntArray::null())
}

fn toplevel_title<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    toplevel_handle: jlong,
) -> Result<JString<'local>, BridgeError> {
    let Some(t) = ptr_to_ref::<state::WinToplevel>(toplevel_handle) else {
        return Ok(JString::null());
    };
    Ok(JString::new(env, &t.title)?)
}

fn toplevel_app_id<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    toplevel_handle: jlong,
) -> Result<JString<'local>, BridgeError> {
    let Some(t) = ptr_to_ref::<state::WinToplevel>(toplevel_handle) else {
        return Ok(JString::null());
    };
    Ok(JString::new(env, &t.app_id)?)
}

fn resize_hwnd(hwnd: windows::Win32::Foundation::HWND, width: i32, height: i32) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE;
        let _ = SetWindowPos(
            hwnd,
            HWND_TOP,
            0,
            0,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn toplevel_resize<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    toplevel_handle: jlong,
    width: jint,
    height: jint,
    _interactive: jboolean,
) -> Result<(), BridgeError> {
    let Some(t) = ptr_to_mut::<state::WinToplevel>(toplevel_handle) else {
        return Err(BridgeError::NullToplevel);
    };
    t.geom_w = width;
    t.geom_h = height;
    t.maximize = false;
    t.fullscreen = false;
    resize_hwnd(t.hwnd, width, height);
    Ok(())
}

fn toplevel_resize_ovr<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    toplevel_handle: jlong,
    width: jint,
    height: jint,
) -> Result<(), BridgeError> {
    toplevel_resize(_env, _class, toplevel_handle, width, height, false)
}

fn toplevel_maximize<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    toplevel_handle: jlong,
) -> Result<(), BridgeError> {
    let state = instance!(instance);
    let bounds = state.output_bounds;
    if let Some(t) = ptr_to_mut::<state::WinToplevel>(toplevel_handle) {
        t.maximize = true;
        t.fullscreen = false;
        t.geom_w = bounds.0;
        t.geom_h = bounds.1;
        resize_hwnd(t.hwnd, bounds.0, bounds.1);
    }
    Ok(())
}

fn toplevel_fullscreen<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    toplevel_handle: jlong,
) -> Result<(), BridgeError> {
    let state = instance!(instance);
    let size = state.output_size;
    if let Some(t) = ptr_to_mut::<state::WinToplevel>(toplevel_handle) {
        t.fullscreen = true;
        t.geom_w = size.0;
        t.geom_h = size.1;
        resize_hwnd(t.hwnd, size.0, size.1);
    }
    Ok(())
}

fn kill_toplevel<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    toplevel_handle: jlong,
) -> Result<jboolean, BridgeError> {
    let state = instance!(instance);
    Ok(process::kill_toplevel(state, toplevel_handle))
}

fn update_surface_data<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
    jsurface: WLCSurface<'local>,
) -> Result<(), BridgeError> {

    let surface_ptr = jsurface.handle(env)?;
    let Some(surface) = ptr_to_ref::<state::WinSurface>(surface_ptr) else {
        eprintln!("[windowmod] update_surface_data: NO WinSurface for ptr=0x{:x} (not registered?)", surface_ptr);
        return Ok(());
    };

    if !surface.buffer_dirty {
        return Ok(());
    }

    let Some(toplevel) = ptr_to_ref::<state::WinToplevel>(surface.toplevel_ptr) else {
        eprintln!("[windowmod] update_surface_data: NO WinToplevel for toplevel_ptr=0x{:x} (dead window?)", surface.toplevel_ptr);
        jsurface.remove_buffer(env).ok();
        return Ok(());
    };

    if toplevel.buffer.is_empty() {
        eprintln!("[windowmod] update_surface_data: toplevel 0x{:x} buffer is empty — cannot attach", surface.toplevel_ptr);
        jsurface.remove_buffer(env).ok();
        return Ok(());
    }

    let width = toplevel.width;
    let height = toplevel.height;
    let stride = width * 4;
    let ptr = toplevel.buffer.as_ref().as_ptr() as jlong;

    jsurface

        .attach_shm_buffer(env, ptr, width, height, 0, stride)
        .map_err(BridgeError::JniError)?;

    jsurface.clear_damage(env).ok();
    for d in &surface.damage {
        jsurface
            .add_surface_damage(env, d[0], d[1], d[2], d[3])
            .ok();
    }
    if let Some(surface) = ptr_to_mut::<state::WinSurface>(surface_ptr) {
        surface.damage.clear();
        surface.buffer_dirty = false;
    }
    Ok(())
}

fn update_surface_tree<'local>(
    env: &mut Env<'local>,
    this: WaylandCraftBridge<'local>,
    instance: jlong,
    surface: WLCSurface<'local>,
) -> Result<WLCSurface<'local>, BridgeError> {
    let handle = surface.handle(env)?;

    // The root surface of a window (toplevel root). Reset its tree links and
    // mark it visited so it is not garbage-collected this frame.
    surface.set_visited(env, true).ok();
    surface.set_parent_handle(env, 0).ok();
    surface.set_prev_child(env, WLCSurface::null()).ok();
    if let Some(s) = ptr_to_ref::<state::WinSurface>(handle) {
        surface.set_xoff(env, s.xoff).ok();
        surface.set_yoff(env, s.yoff).ok();
    }

    // Find every POPUP surface that is parented to THIS root surface. Each such
    // popup must be drawn INSIDE this window, so we splice its surface into the
    // owner's surface chain as a child (Java's WindowFramebuffer walks the
    // `nextChild` chain and draws every surface at its xSubpos/ySubpos offset).
    //
    // We collect the native surface pointers of those children first (immutable
    // borrow of state), then build the Java doubly-linked list.
    let child_ptrs: Vec<(i64, i32, i32)> = {
        let Some(st) = ptr_to_ref::<WindowMod>(instance) else {
            surface.set_next_child(env, WLCSurface::null()).ok();
            return Ok(surface);
        };
        st.surfaces
            .iter()
            .filter(|s| s.parent_ptr == handle)
            .map(|s| (state::ptr_of_ref(&**s), s.xoff, s.yoff))
            .collect()
    };

    if child_ptrs.is_empty() {
        surface.set_next_child(env, WLCSurface::null()).ok();
        return Ok(surface);
    }

    // Build the chain: root -> child0 -> child1 -> ...
    //
    // `prev` owns the previous surface in the chain (starting at the root).
    // Each iteration links prev <-> child, then `child` becomes the new `prev`.
    // We never `.clone()` a WLCSurface (that would clone the inner jobject
    // pointer via Deref, not the wrapper), instead we move owned values like
    // the Linux compositor's update_surface_tree does.
    let mut prev = surface;
    for (child_ptr, xoff, yoff) in child_ptrs {
        let child = this.get_or_create_surface(env, child_ptr)?;
        child.set_visited(env, true).ok();
        child.set_parent_handle(env, handle).ok();
        child.set_xoff(env, xoff).ok();
        child.set_yoff(env, yoff).ok();
        child.set_next_child(env, WLCSurface::null()).ok();
        child.set_prev_child(env, &prev).ok();

        prev.set_next_child(env, &child).ok();

        prev = child;
    }

    Ok(prev)
}




fn surface_xdg_geometry<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    surface_handle: jlong,
) -> Result<JPrimitiveArray<'local, jint>, BridgeError> {
    let Some(surface) = ptr_to_ref::<state::WinSurface>(surface_handle) else {
        return Ok(JIntArray::null());
    };
    let Some(toplevel) = ptr_to_ref::<state::WinToplevel>(surface.toplevel_ptr) else {
        return Ok(JIntArray::null());
    };
    let geom = [
        toplevel.geom_x,
        toplevel.geom_y,
        toplevel.geom_w,
        toplevel.geom_h,
    ];
    let array = JIntArray::new(env, 4)?;
    array.set_region(env, 0, &geom)?;
    Ok(array)
}

fn dmabufs<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<JPrimitiveArray<'local, jlong>, BridgeError> {
    Ok(JLongArray::new(env, 0)?)
}

fn check_input_region<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    surface_handle: jlong,
    x: jdouble,
    y: jdouble,
) -> Result<jboolean, BridgeError> {
    let Some(surface) = ptr_to_ref::<state::WinSurface>(surface_handle) else {
        return Ok(false);
    };
    let Some(toplevel) = ptr_to_ref::<state::WinToplevel>(surface.toplevel_ptr) else {
        return Ok(false);
    };
    let inside = x >= 0.0
        && y >= 0.0
        && x < toplevel.geom_w as f64
        && y < toplevel.geom_h as f64;
    Ok(inside)
}

fn pointer_motion<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    x: jdouble,
    y: jdouble,
) -> Result<(), BridgeError> {
    input::pointer_motion(instance!(instance), x, y);
    Ok(())
}

fn pointer_motion_focus<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    surface_handle: jlong,
    x: jdouble,
    y: jdouble,
) -> Result<(), BridgeError> {
    input::pointer_motion_focus(instance!(instance), surface_handle, x, y);
    Ok(())
}

fn pointer_rel_motion<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    dx: jdouble,
    dy: jdouble,
) -> Result<(), BridgeError> {
    input::pointer_relative_motion(instance!(instance), dx, dy);
    Ok(())
}

fn maybe_pointer_lock<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    surface_handle: jlong,
) -> Result<jboolean, BridgeError> {
    Ok(input::maybe_pointer_lock(instance!(instance), surface_handle))
}

fn pointer_unlock<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<(), BridgeError> {
    input::pointer_unlock(instance!(instance));
    Ok(())
}

fn pointer_leave<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<(), BridgeError> {
    input::pointer_leave(instance!(instance));
    Ok(())
}

fn pointer_button<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    button: jint,
    state: jint,
) -> Result<jint, BridgeError> {
    let pressed = state == 1;
    Ok(input::pointer_button(instance!(instance), button as u32, pressed) as jint)
}

fn pointer_axis<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    axis: jint,
    value: jdouble,
) -> Result<(), BridgeError> {
    input::pointer_axis(instance!(instance), axis, value);
    Ok(())
}

fn cursor_shape<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<jint, BridgeError> {
    Ok(-1)
}

fn keyboard_focus<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    toplevel_handle: jlong,
) -> Result<(), BridgeError> {
    let toplevel = if toplevel_handle == 0 {
        None
    } else {
        Some(toplevel_handle)
    };
    input::focus_toplevel(instance!(instance), toplevel);
    Ok(())
}

fn keyboard_activate<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<(), BridgeError> {
    input::activate_keyboard(instance!(instance));
    Ok(())
}

fn keyboard_deactivate<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<(), BridgeError> {
    input::deactivate_keyboard(instance!(instance));
    Ok(())
}

fn keyboard_input<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    scancode: jint,
    action: jint,
) -> Result<(), BridgeError> {
    input::keyboard_key(instance!(instance), scancode as u32, action == 1);
    Ok(())
}

fn keyboard_update<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    scancode: jint,
    pressed: jboolean,
) -> Result<(), BridgeError> {
    input::keyboard_update(instance!(instance), scancode as u32, pressed);
    Ok(())
}

fn output_size<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jint>, BridgeError> {
    let size = instance!(instance).output_size;
    let array = JIntArray::new(env, 2)?;
    array.set_region(env, 0, &[size.0, size.1])?;
    Ok(array)
}

fn output_bounds<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JPrimitiveArray<'local, jint>, BridgeError> {
    let bounds = instance!(instance).output_bounds;
    let array = JIntArray::new(env, 2)?;
    array.set_region(env, 0, &[bounds.0, bounds.1])?;
    Ok(array)
}

fn output_resize<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    width: jint,
    height: jint,
) -> Result<(), BridgeError> {
    instance!(instance).output_size = (width, height);
    Ok(())
}

fn output_set_bounds<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    width: jint,
    height: jint,
) -> Result<(), BridgeError> {
    instance!(instance).output_bounds = (width, height);
    Ok(())
}

fn free_surface<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    surface_handle: jlong,
) -> Result<(), BridgeError> {
    let state = instance!(instance);
    state.surfaces.retain(|s| state::ptr_of_ref(&**s) != surface_handle);
    Ok(())
}

fn free_toplevel<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    toplevel_handle: jlong,
) -> Result<(), BridgeError> {
    let state = instance!(instance);
    state
        .toplevels
        .retain(|t| state::ptr_of_ref(&**t) != toplevel_handle);
    state
        .surfaces
        .retain(|s| s.toplevel_ptr != toplevel_handle);
    Ok(())
}

fn free_popup<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
    _popup_handle: jlong,
) -> Result<(), BridgeError> {
    Ok(())
}

fn raw_to_java<'local>(
    env: &mut Env<'local>,
    entry: &RawDesktopEntry,
) -> Result<JRawDesktopEntry<'local>, BridgeError> {
    let app_id = JString::new(env, &entry.app_id)?;
    let name = match &entry.name {
        Some(s) => JString::new(env, s)?,
        None => JString::null(),
    };
    let generic_name = match &entry.generic_name {
        Some(s) => JString::new(env, s)?,
        None => JString::null(),
    };
    let exec = match &entry.exec {
        Some(s) => JString::new(env, s)?,
        None => JString::null(),
    };
    let comment = match &entry.comment {
        Some(s) => JString::new(env, s)?,
        None => JString::null(),
    };
    let icon_path = match &entry.icon_path {
        Some(s) => JString::new(env, s)?,
        None => JString::null(),
    };

    let keywords: Vec<_> = entry
        .keywords
        .iter()
        .map(|k| JString::new(env, k))
        .collect::<Result<_, _>>()?;
    let kw_array =
        JObjectArray::<JString>::new(env, keywords.len(), &JString::null())?;
    for (i, k) in keywords.iter().enumerate() {
        kw_array.set_element(env, i, k)?;
    }

    let categories: Vec<_> = entry
        .categories
        .iter()
        .map(|c| JString::new(env, c))
        .collect::<Result<_, _>>()?;
    let cat_array =
        JObjectArray::<JString>::new(env, categories.len(), &JString::null())?;
    for (i, c) in categories.iter().enumerate() {
        cat_array.set_element(env, i, c)?;
    }

    Ok(JRawDesktopEntry::new(
        env,
        app_id,
        name,
        generic_name,
        exec,
        entry.exec_terminal,
        comment,
        kw_array,
        cat_array,
        entry.visible,
        icon_path,
    )?)
}

fn load_desktop_entry<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    path: JString<'local>,
) -> Result<JRawDesktopEntry<'local>, BridgeError> {
    let state = instance!(instance);
    let path: PathBuf = path.try_to_string(env)?.into();
    if let Some(app) = apps::parse_lnk_file(&path) {
        return raw_to_java(env, &apps::to_raw(&app));
    }
    for app in &state.desktop_apps {
        if app.exec.as_ref().map(|e| PathBuf::from(e)) == Some(path.clone()) {
            return raw_to_java(env, &apps::to_raw(app));
        }
    }
    Ok(JRawDesktopEntry::null())
}

fn load_desktop_entries<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
) -> Result<JObjectArray<'local, JRawDesktopEntry<'local>>, BridgeError> {
    let state = instance!(instance);
    let entries: Vec<_> = state
        .desktop_apps
        .iter()
        .map(|e| raw_to_java(env, &apps::to_raw(e)))
        .collect::<Result<_, _>>()?;
    let array = JObjectArray::<JRawDesktopEntry>::new(
        env,
        entries.len(),
        &JRawDesktopEntry::null(),
    )?;
    for (i, e) in entries.iter().enumerate() {
        array.set_element(env, i, e)?;
    }
    Ok(array)
}

fn render_svg<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
    width: jint,
    height: jint,
    buffer_ptr: jlong,
) -> Result<jboolean, BridgeError> {
    let path: PathBuf = path.try_to_string(env)?.into();
    let data = buffer_ptr as usize as *mut u8;
    Ok(if crate::svg::render_svg(path, width as u32, height as u32, data).is_some() {
        true
    } else {
        false
    })
}

fn render_image<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    path: JString<'local>,
    width: jint,
    height: jint,
    buffer_ptr: jlong,
) -> Result<jboolean, BridgeError> {
    let path: String = path.try_to_string(env)?;
    let data = buffer_ptr as usize as *mut u8;
    let data_slice = unsafe { std::slice::from_raw_parts_mut(data, (width as usize) * (height as usize) * 4) };

    let img = match image::open(&path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("[windowmod] render_image: failed to open {}: {}", path, e);
            return Ok(false);
        }
    };

    let rgba = img.resize_exact(width as u32, height as u32, image::imageops::FilterType::Lanczos3).to_rgba8();
    for (i, pixel) in rgba.pixels().enumerate() {
        let offset = i * 4;
        data_slice[offset] = pixel[0];
        data_slice[offset + 1] = pixel[1];
        data_slice[offset + 2] = pixel[2];
        data_slice[offset + 3] = pixel[3];
    }
    eprintln!("[windowmod] render_image: loaded {} ({}x{})", path, width, height);
    Ok(true)
}

fn exec_app<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    app_id: JString<'local>,
) -> Result<jboolean, BridgeError> {
    let app_id = app_id.try_to_string(env)?;
    eprintln!("[windowmod] bridge::exec_app called with app_id='{}'", app_id);
    let result = process::spawn_app(instance!(instance), &app_id);
    eprintln!("[windowmod] bridge::exec_app result: {}", result);
    Ok(result)
}

fn launch_exe<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    path: JString<'local>,
) -> Result<jboolean, BridgeError> {
    let path = path.try_to_string(env)?;
    eprintln!("[windowmod] bridge::launch_exe called with path='{}'", path);
    let state = instance!(instance);

    let working_dir = std::path::Path::new(path.as_str())
        .parent()
        .map(|p| p.to_string_lossy().into_owned());

    let app = apps::DesktopApp {
        app_id: path.clone(),
        name: None,
        generic_name: None,
        exec: Some(path.clone()),
        exec_args: None,
        working_dir,
        exec_terminal: false,
        comment: None,
        keywords: Vec::new(),
        categories: Vec::new(),
        visible: true,
        icon_path: None,
    };
    let result = process::spawn_desktop_app(state, &app);
    eprintln!("[windowmod] bridge::launch_exe result: {}", result);
    Ok(result)
}

fn set_preferred_terminal<'local>(
    env: &mut Env<'local>,
    _class: JClass<'local>,
    instance: jlong,
    cmd: JString<'local>,
) -> Result<(), BridgeError> {
    instance!(instance).preferred_terminal = cmd.try_to_string(env)?;
    Ok(())
}

fn set_keymap_default<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<(), BridgeError> {
    Ok(())
}

fn export_keymap<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<JString<'local>, BridgeError> {
    Ok(JString::null())
}

fn set_keymap_from_str<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
    _keymap: JString<'local>,
) -> Result<jboolean, BridgeError> {
    Ok(false)
}

fn check_dnd_request<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<JPrimitiveArray<'local, jint>, BridgeError> {
    Ok(JIntArray::null())
}

fn check_dnd_active<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<jboolean, BridgeError> {
    Ok(false)
}

fn dnd_cancel<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<(), BridgeError> {
    Ok(())
}

fn dnd_drop<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<(), BridgeError> {
    Ok(())
}

fn dnd_motion<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
    _surface_handle: jlong,
    _x: jdouble,
    _y: jdouble,
) -> Result<(), BridgeError> {
    Ok(())
}

fn dnd_icon<'local>(
    _env: &mut Env<'local>,
    _class: JClass<'local>,
    _instance: jlong,
) -> Result<jlong, BridgeError> {
    Ok(0)
}
