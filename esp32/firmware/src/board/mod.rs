//! Pins and panel of the supported boards. Shared values are here, the
//! differences in one module per board:
//!
//! * `waveshare` (default): Waveshare ESP32-P4-WIFI6-Touch-LCD-5
//! * with the Cargo feature `guition`: Guition JC4880P443C_I_W
//!
//! Buttons and battery gauge (below) describe the Waveshare board's 40-pin
//! header; both are off on both boards.

#[cfg(not(feature = "guition"))]
mod waveshare;
#[cfg(not(feature = "guition"))]
pub use waveshare::*;

#[cfg(feature = "guition")]
mod guition;
#[cfg(feature = "guition")]
pub use guition::*;

/// I²C-Bus 1 (wie im Board-Support-Paket, 400 kHz).
pub const I2C_PORT: i32 = 1;
pub const I2C_SDA: i32 = 7;
pub const I2C_SCL: i32 = 8;
pub const I2C_HZ: u32 = 400_000;

/// ES8311 (`ES8311_CODEC_DEFAULT_ADDR`, 7 Bit).
pub const ADDR_CODEC: u8 = 0x18;
/// GT911: Standard- und Ersatzadresse (wie im Board-Support-Paket geprüft).
pub const ADDR_TOUCH: [u8; 2] = [0x5D, 0x14];

/// Touch abfragen: alle 4 ms (250-mal pro Sekunde, eigener Thread). Der GT911
/// liefert selbst neue Daten etwa alle 10 ms; so geht keine Messung verloren
/// und kurzes Antippen zwischen zwei Bildern wird trotzdem erkannt.
pub const TOUCH_POLL_MS: u32 = 4;
/// I²S zum ES8311.
pub const I2S_MCLK: i32 = 13;
pub const I2S_BCLK: i32 = 12;
pub const I2S_WS: i32 = 10;
pub const I2S_DOUT: i32 = 9;
pub const DSI_LANES: u8 = 2;
pub const DSI_PHY_LDO_CHAN: i32 = 3;
pub const DSI_PHY_LDO_MV: i32 = 2500;

/// Nominal refresh rate of the panel; the firmware measures the real one.
pub const PANEL_HZ: f32 = DPI_CLOCK_MHZ * 1.0e6
    / (((LCD_W as u32 + HSYNC_PULSE + HSYNC_BACK + HSYNC_FRONT)
        * (LCD_H as u32 + VSYNC_PULSE + VSYNC_BACK + VSYNC_FRONT)) as f32);
/// Time budget per frame at that rate.
pub const FRAME_BUDGET_MS: f32 = 1000.0 / PANEL_HZ;

/// Knöpfe an der Leiste – **aus**, solange nur per Touch gespielt wird.
/// Nach dem Anlöten auf `true` setzen (Pins siehe unten und oben).
pub const BUTTONS_ENABLED: bool = false;
/// Knöpfe (aktiv low, interner Pull-up): GPIO 48–52.
pub const BTN_UP: i32 = 49;
pub const BTN_DOWN: i32 = 50;
pub const BTN_LEFT: i32 = 51;
pub const BTN_RIGHT: i32 = 52;
pub const BTN_JUMP: i32 = 48;
/// BOOT-Knopf als Pause/Zurück – erst eintragen, wenn `hwtest` bestätigt
/// hat, an welchem GPIO er hängt (siehe oben). Wirkt auch ohne
/// [`BUTTONS_ENABLED`].
pub const BTN_MENU: Option<i32> = None;
/// Wird von `hwtest` angezeigt, um den BOOT-Knopf zu finden.
pub const BOOT_CANDIDATE: i32 = 35;

/// Akkuspannung über einen Spannungsteiler an einem ADC-Pin.
#[derive(Clone, Copy, Debug)]
pub struct BatterySense {
    pub gpio: i32,
    /// Akkuspannung / Pinspannung, z. B. (200 kΩ + 100 kΩ) / 100 kΩ = 3.
    pub divider: f32,
}

/// Vorschlag (siehe README, „Akkuanzeige“): GPIO 21 = Leisten-Pin 15,
/// ADC1 Kanal 5; Akku+ → 200 kΩ → Pin 15 → 100 kΩ → GND (Pin 13),
/// 100 nF parallel zu den 100 kΩ. Bei 4,2 V liegen am Pin 1,4 V.
pub const BATTERY_SUGGESTED: BatterySense = BatterySense { gpio: 21, divider: 3.0 };

/// Akkuanzeige – **aus**, bis der Spannungsteiler eingelötet ist. Danach:
/// `Some(BATTERY_SUGGESTED)` (und `divider` ggf. mit einem Multimeter
/// nachjustieren: 3,0 × gemessene Akkuspannung ÷ angezeigte Spannung).
pub const BATTERY: Option<BatterySense> = None;
