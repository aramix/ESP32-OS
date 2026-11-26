//! Pins of the Guition JC4880P443C_I_W, taken from the vendor's documents
//! (schematic, ESP-IDF examples including the ST7701 driver).
//!
//! | Function                 | GPIO / value                                  |
//! |--------------------------|-----------------------------------------------|
//! | I²C SDA / SCL            | 7 / 8 (GT911 touch, ES8311 codec, camera)     |
//! | I²S MCLK / BCLK / LRCK   | 13 / 12 / 10                                  |
//! | I²S DOUT (to the codec)  | 9                                             |
//! | Amplifier (PA_CTRL)      | 11 (high = on)                                |
//! | LCD reset                | 5 (active low)                                |
//! | LCD backlight            | 23 (PWM 5 kHz, 10 bit)                        |
//! | MIPI-DSI                 | 2 lanes at 750 Mbit/s, PHY supply LDO 3 2.5 V |
//! | Touch reset / interrupt  | 22 / 21 (not used here)                       |
//! | microSD                  | SDMMC slot 0 (39-44), supply LDO 4            |
//! | ESP32-C6 (SDIO, reset)   | 14-19, 54                                     |
//!
//! **Buttons and battery gauge** are off: the board is operated by touch.

use crate::hw::zeroed;
use esp_idf_svc::sys::{self, EspError, esp};

/// No updates over Wi-Fi: the ESP32-OS releases are built for the Waveshare
/// board.
pub const UPDATE_REPO: Option<&str> = None;
/// Black: the PPA never writes the bars next to the picture.
pub const BOOT_COLOR: u32 = 0x000000;

/// Touch interrupt: GPIO 21 according to the schematic, not tried yet. So it
/// is off and the driver polls every [`TOUCH_POLL_MS`]. `hwtest` counts
/// whether GPIO 21 toggles on touch; only then set `Some(21)` here.
pub const TOUCH_INT: Option<i32> = None;
/// Watched by `hwtest` (see [`TOUCH_INT`]).
pub const TOUCH_INT_CANDIDATE: i32 = 21;

/// Amplifier enable (high = on).
pub const PA_EN: i32 = 11;

/// Display: ST7701, 480×800 portrait, MIPI-DSI.
pub const LCD_W: usize = 480;
pub const LCD_H: usize = 800;
pub const LCD_RST: i32 = 5;
pub const LCD_BACKLIGHT: i32 = 23;
pub const DSI_LANE_MBPS: u32 = 750;
/// Timing as in the vendor demo: 34 MHz pixel clock for
/// (480+12+42+42) × (800+2+8+166) = 576 × 976 clocks, nominally 60.5 frames
/// per second.
pub const DPI_CLOCK_MHZ: f32 = 34.0;
pub const HSYNC_PULSE: u32 = 12;
pub const HSYNC_BACK: u32 = 42;
pub const HSYNC_FRONT: u32 = 42;
pub const VSYNC_PULSE: u32 = 2;
pub const VSYNC_BACK: u32 = 8;
pub const VSYNC_FRONT: u32 = 166;
/// ST7701 driver with this panel's init sequence
/// (komponenten/esp_lcd_st7701, from Guition's demo).
///
/// # Safety
/// `bus` and `io` belong to the DSI bus; `dpi` lives until the call returns.
pub unsafe fn create_panel(
    bus: sys::esp_lcd_dsi_bus_handle_t,
    io: sys::esp_lcd_panel_io_handle_t,
    dpi: &sys::esp_lcd_dpi_panel_config_t,
) -> Result<sys::esp_lcd_panel_handle_t, EspError> {
    let mut vendor: sys::st7701_vendor_config_t = zeroed();
    vendor.__bindgen_anon_1.mipi_config.dsi_bus = bus;
    vendor.__bindgen_anon_1.mipi_config.dpi_config = dpi;
    vendor.flags.set_use_mipi_interface(1);
    let mut dev: sys::esp_lcd_panel_dev_config_t = zeroed();
    dev.reset_gpio_num = LCD_RST;
    dev.bits_per_pixel = 16;
    dev.vendor_config = (&mut vendor as *mut sys::st7701_vendor_config_t).cast();
    let mut panel = core::ptr::null_mut();
    // SAFETY: see above; the configurations live during the call.
    esp!(unsafe { sys::esp_lcd_new_panel_st7701(io, &dev, &mut panel) })?;
    Ok(panel)
}
