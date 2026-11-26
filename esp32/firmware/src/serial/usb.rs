//! The P4's USB Serial/JTAG port (Guition).

use esp_idf_svc::sys::{self, EspError, esp};

/// USB ignores the baud rate; the screenshot header names one only because
/// `bildschirmfoto.py` expects it.
pub const SCREENSHOT_BAUD: u32 = 115_200;

/// Install the driver with its ring buffer (once, at startup).
pub fn init() -> Result<(), EspError> {
    let mut cfg = sys::usb_serial_jtag_driver_config_t { tx_buffer_size: 8192, rx_buffer_size: 256 };
    // SAFETY: once at startup; the configuration is valid.
    esp!(unsafe { sys::usb_serial_jtag_driver_install(&mut cfg) })?;
    // ESP_LOG and printf go through the same driver.
    // SAFETY: the driver is installed.
    unsafe { sys::usb_serial_jtag_vfs_use_driver() };
    Ok(())
}

/// Copy `n` bytes from `buf` into the ring buffer without waiting: if it is
/// full (no PC is reading), they are dropped instead of stalling the system.
pub fn write(buf: *const u8, n: usize) {
    // SAFETY: `buf` is valid for `n` bytes; the driver copies them.
    unsafe { sys::usb_serial_jtag_write_bytes(buf.cast(), n as _, 0) };
}

/// Wait briefly until everything has been sent.
pub fn flush() {
    // SAFETY: the driver is installed.
    unsafe { sys::usb_serial_jtag_wait_tx_done(10) };
}

/// Send everything; `false` if nobody reads for 1 s.
fn send_all(mut rest: &[u8]) -> bool {
    while !rest.is_empty() {
        // SAFETY: valid buffer; the driver is installed.
        let n = unsafe { sys::usb_serial_jtag_write_bytes(rest.as_ptr().cast(), rest.len() as _, 1000) };
        if n <= 0 {
            return false;
        }
        rest = &rest[n as usize..];
    }
    true
}

/// Send image data with its marker and checksum.
pub fn send_image(data: &[u8]) {
    // SAFETY: the driver is installed.
    unsafe {
        sys::usb_serial_jtag_wait_tx_done(1000);
        sys::vTaskDelay(300);
    }
    let sum = data.iter().fold(0u32, |s, &b| s.wrapping_add(b as u32));
    if !(send_all(b"BILDDATA") && data.chunks(2048).all(send_all) && send_all(&sum.to_le_bytes())) {
        log::warn!("screenshot aborted: nobody is reading");
    }
    // SAFETY: as above.
    unsafe {
        sys::usb_serial_jtag_wait_tx_done(5000);
        sys::vTaskDelay(300);
    }
}
