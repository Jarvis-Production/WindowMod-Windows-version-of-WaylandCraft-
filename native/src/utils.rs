#[cfg(not(target_os = "linux"))]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(target_os = "linux")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use smithay::utils::SERIAL_COUNTER;

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
static WINDOWS_SERIAL: AtomicU32 = AtomicU32::new(1);

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub fn new_serial() -> u32 {
    WINDOWS_SERIAL.fetch_add(1, Ordering::Relaxed)
}

#[cfg(target_os = "linux")]
pub fn new_serial() -> u32 {
    SERIAL_COUNTER.next_serial().into()
}

#[cfg(target_os = "linux")]
pub fn get_time() -> u32 {
    let time: u128 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    time as u32
}

#[cfg(target_os = "linux")]
pub fn to_fixed(v: f64) -> i32 {
    (v * 256.0) as i32
}

#[cfg(target_os = "linux")]
pub fn to_fixed2(v1: f64, v2: f64) -> (i32, i32) {
    (to_fixed(v1), to_fixed(v2))
}
