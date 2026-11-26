//! Screenshots of the displayed frame buffer over the serial line, for
//! development (build with `KONSOLE_BILDER=10,25`: seconds after startup).
//! The PC side is `bildschirmfoto.py`.
//!
//! Protocol: the header `BILD <no> <width> <height> <baud> <flipped>` at
//! 115200 baud, then `BILDDATA`, the pixels (RGB565, little endian, panel in
//! portrait) and a 32-bit sum of all bytes (over UART at `baud`, see
//! [`crate::serial`]). Other log output is muted meanwhile.

use crate::board::{LCD_H, LCD_W};
use crate::display::FB_BYTES;
use crate::serial::{SCREENSHOT_BAUD, send_image};
use esp_idf_svc::sys;

/// Send the frame buffer `fb` (blocks for a few seconds).
pub fn senden(fb: *const u16, nummer: u32, gedreht: bool) {
    log::info!("BILD {nummer} {LCD_W} {LCD_H} {SCREENSHOT_BAUD} {}", gedreht as u8);
    crate::hw::LOG_STUMM.store(true, core::sync::atomic::Ordering::Release);
    // SAFETY: `fb` is a whole frame buffer that the PPA is not writing to (it
    // is on screen). The PPA wrote it by DMA, so drop stale cache lines.
    let data = unsafe {
        sys::esp_cache_msync(fb as *mut _, FB_BYTES, crate::hw::CACHE_MSYNC_DIR_M2C as _);
        core::slice::from_raw_parts(fb as *const u8, FB_BYTES)
    };
    send_image(data);
    crate::hw::LOG_STUMM.store(false, core::sync::atomic::Ordering::Release);
    log::info!("BILD {nummer} gesendet");
}
