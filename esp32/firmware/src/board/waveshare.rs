//! Pinbelegung des Waveshare ESP32-P4-WIFI6-Touch-LCD-5 – nur Werte aus der
//! Dokumentation des Herstellers:
//!
//! * Waveshare-Wiki, Tabelle der 40-Pin-Leiste
//!   (<https://docs.waveshare.com/ESP32-P4-WIFI6-Touch-LCD-5>)
//! * Waveshare-Repository `ESP32-P4-WIFI6-Touch-LCD-5`, `docs/IO.md`
//! * Board-Support-Paket `waveshare/esp32_p4_wifi6_touch_lcd_5` 1.0.4
//!   (`include/bsp/*.h`) und Panel-Treiber `waveshare/esp_lcd_hx8394` 2.1
//! * ADC-Kanäle: ESP-IDF `soc/esp32p4/include/soc/adc_channel.h`
//!
//! | Funktion                 | GPIO / Wert                                   |
//! |--------------------------|-----------------------------------------------|
//! | I²C SDA / SCL            | 7 / 8 (Touch GT911, Codec ES8311, ES7210)      |
//! | I²S MCLK / BCLK / LRCK   | 13 / 12 / 10                                  |
//! | I²S DOUT (zum Codec)     | 9                                             |
//! | Verstärker (PA)          | 53 (high = an)                                |
//! | LCD Reset                | 27 (im BSP: `reset_active_high = 1`)          |
//! | LCD Hintergrundlicht     | 26 (PWM 5 kHz, 10 Bit)                        |
//! | MIPI-DSI                 | 2 Lanes à 700 Mbit/s, PHY-Spannung LDO 3 2,5 V |
//! | Touch Reset / Interrupt  | im BSP nicht benutzt (Schaltplan laut `IO.md`: 23 / 2, Interrupt nur über einen optionalen Widerstand) |
//!
//! ## 40-Pin-Leiste (Auszug aus der Waveshare-Tabelle)
//!
//! Die Leiste ist **nicht** wie beim Raspberry Pi belegt (Pin 1 = 5 V,
//! Pin 2 = 3V3). Wo Pin 1 sitzt, sagt die Tabelle nicht – laut Waveshare vor
//! dem Verdrahten am Platinenaufdruck prüfen.
//!
//! | Leisten-Pin | Signal | hier benutzt für |
//! |-------------|--------|------------------|
//! | 13          | GND    | Akkuanzeige (GND) |
//! | 15          | GPIO21 | Akkuanzeige (ADC1 Kanal 5) |
//! | 30          | GPIO35 | BOOT? (nur Anzeige in `hwtest`) |
//! | 32          | GPIO49 | Knopf Vorne      |
//! | 33          | GND    | Knöpfe (GND)     |
//! | 34          | GPIO50 | Knopf Hinten     |
//! | 36          | GPIO51 | Knopf Links      |
//! | 38          | GPIO52 | Knopf Rechts     |
//! | 39          | GPIO48 | Knopf Springen   |
//! | 40          | GND    | Knöpfe (GND)     |
//!
//! **Knöpfe:** optional, standardmäßig aus ([`BUTTONS_ENABLED`]). Die Konsole
//! ist komplett per Touch bedienbar. Die fünf Knöpfe liegen auf
//! **GPIO 48, 49, 50, 51, 52** (je zwischen Pin und GND, interner Pull-up).
//! Gemieden: 34–38 (Boot-Strapping des ESP32-P4), 24/25 (USB), 2 (evtl.
//! Touch-Interrupt), 7/8 (I²C), 21 (Akkuanzeige).
//!
//! **BOOT-Knopf:** Die Waveshare-Unterlagen nennen seinen GPIO nicht. Er ist
//! daher **nicht** belegt (Pause per Touch oben links). `hwtest` zeigt den
//! Pegel von GPIO 35 (Boot-Strapping-Pin des ESP32-P4): Wechselt er beim
//! Drücken von BOOT, kann [`BTN_MENU`] auf `Some(35)` gesetzt werden.
//!
//! **Akku:** Die Platine lädt den Akku selbst, misst die Spannung aber nicht
//! an einem GPIO. Optional: eigener Spannungsteiler an GPIO 21 ([`BATTERY`]).

use crate::hw::zeroed;
use esp_idf_svc::sys::{self, EspError, esp};

/// Release repository for updates over Wi-Fi.
pub const UPDATE_REPO: Option<&str> = Some("TheUnkn0wnC0DE/ESP32-OS");
/// Screen colour at startup, until the first frame arrives.
pub const BOOT_COLOR: u32 = 0x2A1030;

/// Touch-Interrupt: laut `IO.md` an GPIO 2, aber nur über einen optionalen
/// Widerstand – ob er bestückt ist, ist nicht dokumentiert. Daher aus; der
/// Treiber fragt dann im Takt [`TOUCH_POLL_MS`] ab. `hwtest` zählt, ob GPIO 2
/// beim Berühren wechselt; nur dann hier `Some(2)` eintragen.
pub const TOUCH_INT: Option<i32> = None;
/// Wird von `hwtest` beobachtet (siehe [`TOUCH_INT`]).
pub const TOUCH_INT_CANDIDATE: i32 = 2;

/// Amplifier enable (high = on).
pub const PA_EN: i32 = 53;

/// Display: HX8394, 720×1280 hochkant, MIPI-DSI.
pub const LCD_W: usize = 720;
pub const LCD_H: usize = 1280;
pub const LCD_RST: i32 = 27;
pub const LCD_BACKLIGHT: i32 = 26;
pub const DSI_LANE_MBPS: u32 = 700;
/// Zeitverhalten aus `HX8394_720_1280_PANEL_30HZ_DPI_CONFIG` des
/// Waveshare-Treibers: 58 MHz Pixeltakt bei (720+20+20+40) × (1280+4+10+24)
/// = 800 × 1318 Takten → rechnerisch 55,0 Bilder pro Sekunde.
pub const DPI_CLOCK_MHZ: f32 = 58.0;
pub const HSYNC_PULSE: u32 = 20;
pub const HSYNC_BACK: u32 = 20;
pub const HSYNC_FRONT: u32 = 40;
pub const VSYNC_PULSE: u32 = 4;
pub const VSYNC_BACK: u32 = 10;
pub const VSYNC_FRONT: u32 = 24;
/// Waveshare's HX8394 driver (sends the init sequence).
///
/// # Safety
/// `bus` and `io` belong to the DSI bus; `dpi` lives until the call returns.
pub unsafe fn create_panel(
    bus: sys::esp_lcd_dsi_bus_handle_t,
    io: sys::esp_lcd_panel_io_handle_t,
    dpi: &sys::esp_lcd_dpi_panel_config_t,
) -> Result<sys::esp_lcd_panel_handle_t, EspError> {
    let mut vendor: sys::hx8394_vendor_config_t = zeroed();
    vendor.mipi_config.dsi_bus = bus;
    vendor.mipi_config.dpi_config = dpi;
    vendor.mipi_config.lane_num = super::DSI_LANES;
    let mut dev: sys::esp_lcd_panel_dev_config_t = zeroed();
    dev.reset_gpio_num = LCD_RST;
    dev.bits_per_pixel = 16;
    dev.flags.set_reset_active_high(1);
    dev.vendor_config = (&mut vendor as *mut sys::hx8394_vendor_config_t).cast();
    let mut panel = core::ptr::null_mut();
    // SAFETY: see above; the configurations live during the call.
    esp!(unsafe { sys::esp_lcd_new_panel_hx8394(io, &dev, &mut panel) })?;
    Ok(panel)
}
