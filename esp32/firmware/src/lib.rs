//! Hardware layer for the supported boards (see [`board`]), shared by the game
//! (`src/main.rs`) and the test program (`src/bin/hwtest.rs`). Drivers come
//! from ESP-IDF (through `esp-idf-svc`).

pub mod absturz;
pub mod audio;
pub mod battery;
pub mod board;
pub mod codec;
pub mod daten;
pub mod display;
pub mod durchsatz;
pub mod seitenliste;
pub mod flash_sperre;
pub mod doom_board;
pub mod foto;
pub mod hw;
pub mod i2c;
pub mod jpeg;
pub mod knopfsuche;
pub mod kern_board;
pub mod serial;
pub mod pipeline;
pub mod slots;
pub mod speicher;
pub mod speicherwacht;
#[cfg(feature = "quake")]
pub mod quake_board;
pub mod store;
pub mod stromtest;
#[cfg(esp32os_testbilder)]
pub mod testbilder;
pub mod touch;
pub mod umzug;
pub mod update;
pub mod wlan;

// The `guition` feature and the ESP-IDF settings must match (console and flash
// size), or the board would stay silent or not boot.
#[cfg(all(feature = "guition", not(esp_idf_esp_console_usb_serial_jtag)))]
compile_error!("--features guition needs sdkconfig.defaults.guition (ESP_IDF_SDKCONFIG_DEFAULTS, see build_flash.sh)");
#[cfg(all(not(feature = "guition"), esp_idf_esp_console_usb_serial_jtag))]
compile_error!("sdkconfig.defaults.guition is only for --features guition");

use board::*;
use esp_idf_svc::sys::EspError;
use pudding_core::input::btn;

/// Spielknöpfe (falls [`board::BUTTONS_ENABLED`]) und – falls bestätigt –
/// BOOT als Eingänge mit Pull-up.
pub fn init_buttons() -> Result<(), EspError> {
    if BUTTONS_ENABLED {
        hw::input_pullup(&[BTN_UP, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_JUMP])?;
    }
    if let Some(pin) = BTN_MENU {
        hw::input_pullup(&[pin])?;
    }
    Ok(())
}

/// Gedrückte Knöpfe als Bitmaske aus `pudding_core::input::btn` (0, solange
/// die Knöpfe ausgeschaltet sind).
pub fn read_buttons() -> u8 {
    let pins = [
        (BTN_UP, btn::UP),
        (BTN_DOWN, btn::DOWN),
        (BTN_LEFT, btn::LEFT),
        (BTN_RIGHT, btn::RIGHT),
        (BTN_JUMP, btn::JUMP),
    ];
    let mut b = if BUTTONS_ENABLED {
        pins.into_iter().fold(0, |b, (pin, bit)| if hw::is_low(pin) { b | bit } else { b })
    } else {
        0
    };
    if let Some(pin) = BTN_MENU
        && hw::is_low(pin)
    {
        b |= btn::MENU;
    }
    b
}

/// Startmeldung: Chip, Takt, Speicher.
pub fn log_chip() {
    let (rev, cores, mhz, psram_mb) = hw::chip();
    let (int_kb, psram_kb) = hw::free_kb();
    log::info!(
        "ESP32-P4 Revision v{}.{}, {} Kerne, {} MHz, PSRAM {} MB; frei: {} KB intern, {} KB PSRAM",
        rev / 100,
        rev % 100,
        cores,
        mhz,
        psram_mb,
        int_kb,
        psram_kb
    );
}
