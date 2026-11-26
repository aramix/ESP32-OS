//! UART0 on the USB-to-UART bridge (Waveshare, "USB TO UART", COM3).

use esp_idf_svc::sys::{self, EspError, esp};

/// Baud rate used while sending a screenshot:
/// 1.8 MB in ~6 s (160 s at 115200 baud).
pub const SCREENSHOT_BAUD: u32 = 3_000_000;
const NORMAL_BAUD: u32 = 115_200;

/// Install the driver with its ring buffer (once, at startup).
pub fn init() -> Result<(), EspError> {
    // SAFETY: once at startup; UART0 is the console.
    unsafe { esp!(sys::uart_driver_install(0, 256, 8192, 0, core::ptr::null_mut(), 0)) }
}

/// Copy `n` bytes from `buf` into the ring buffer (waits only when it is full).
pub fn write(buf: *const u8, n: usize) {
    // SAFETY: `buf` is valid for `n` bytes; the driver copies them.
    unsafe { sys::uart_write_bytes(0, buf.cast(), n) };
}

/// Wait briefly until everything has been sent.
pub fn flush() {
    // SAFETY: the driver is installed.
    unsafe { sys::uart_wait_tx_done(0, 100) };
}

/// Send image data with its marker and checksum (blocks for a few seconds).
pub fn send_image(data: &[u8]) {
    // SAFETY: UART0 has its driver (`init`); the buffers are valid.
    unsafe {
        sys::uart_wait_tx_done(0, 1000);
        sys::vTaskDelay(300);
        sys::uart_set_baudrate(0, SCREENSHOT_BAUD);
        sys::vTaskDelay(50);
        sys::uart_write_bytes(0, b"BILDDATA".as_ptr().cast(), 8);
        let mut sum = 0u32;
        for chunk in data.chunks(4096) {
            sum = chunk.iter().fold(sum, |s, &b| s.wrapping_add(b as u32));
            sys::uart_write_bytes(0, chunk.as_ptr().cast(), chunk.len());
        }
        sys::uart_write_bytes(0, sum.to_le_bytes().as_ptr().cast(), 4);
        sys::uart_wait_tx_done(0, 5000);
        sys::vTaskDelay(100);
        sys::uart_set_baudrate(0, NORMAL_BAUD);
        sys::vTaskDelay(300);
    }
}
