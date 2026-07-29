mod java_types;
mod svg;
mod utils;

#[cfg(target_os = "linux")]
mod compositor;
#[cfg(target_os = "linux")]
mod bridge;
#[cfg(target_os = "linux")]
mod ddm;
#[cfg(target_os = "linux")]
mod egl;
#[cfg(target_os = "linux")]
mod output;
#[cfg(target_os = "linux")]
mod process;
#[cfg(target_os = "linux")]
mod satellite;
#[cfg(target_os = "linux")]
mod seat;
#[cfg(target_os = "linux")]
mod xdg_spec;

#[cfg(target_os = "linux")]
pub use compositor::WaylandCraft;
#[cfg(target_os = "linux")]
pub use xdg_spec::RawDesktopEntry;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
compile_error!("waylandcraft native library supports Linux and Windows only");
