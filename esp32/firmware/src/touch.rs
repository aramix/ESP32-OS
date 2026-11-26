//! GT911 touch through the ESP-IDF driver `espressif/esp_lcd_touch_gt911`
//! (address 0x5D, else 0x14; reset not used; panel coordinates without
//! swapping or mirroring).
//!
//! [`TouchReader`] fragt den Controller in einem eigenen Thread ab – alle
//! [`TOUCH_POLL_MS`] ms oder, falls [`TOUCH_INT`] eingetragen ist, sobald der
//! Interrupt kommt. Die Spielschleife holt sich pro Bild nur den letzten Stand;
//! ein Antippen, das zwischen zwei Bildern schon wieder vorbei ist, wird einmal
//! nachgereicht und geht so nicht verloren.
//!
//! Liefert Panel-Koordinaten (hochkant); die Umrechnung auf das quer liegende
//! Spielbild macht `pudding_core::input::panel_to_game`.

use crate::board::{ADDR_TOUCH, I2C_HZ, LCD_H, LCD_W, TOUCH_INT, TOUCH_POLL_MS};
use crate::hw::{self, spawn_pinned, zeroed};
use crate::i2c::Bus;
use core::sync::atomic::{AtomicBool, Ordering};
use esp_idf_svc::sys::{self, EspError, esp};
use pudding_core::input::MAX_TOUCH;
use std::sync::{Arc, Mutex};

pub type Points = [Option<(u16, u16)>; MAX_TOUCH];

/// Diagnose V1 (`ESP32OS_TOUCH_ROH=1` beim Bauen): Rohregister des GT911
/// und das Ergebnis des Treibers bei jeder Änderung ins Log.
const ROH: bool = option_env!("ESP32OS_TOUCH_ROH").is_some();

/// Ohne GT911-Meldung so lange bei gesetztem Punkt → losgelassen (der GT911
/// meldet bei liegendem Finger in jedem Messzyklus, ca. alle 10 ms).
const LOSGELASSEN_US: i64 = 250_000;

/// Der Touch-Interrupt ist gekommen (nur mit [`TOUCH_INT`]).
static INT_SEEN: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn on_touch_int(_tp: sys::esp_lcd_touch_handle_t) {
    INT_SEEN.store(true, Ordering::Release);
}

pub struct Touch {
    /// Treiber (Einrichtung und Konfiguration des GT911; gelesen wird selbst)
    _tp: sys::esp_lcd_touch_handle_t,
    /// I2C-Kanal zum GT911 (Rohregister, Diagnose V1)
    io: sys::esp_lcd_panel_io_handle_t,
    /// Diagnose: zuletzt protokollierter Rohstand / gemeldeter Stand
    roh_alt: Vec<u8>,
    treiber_alt: Points,
    /// Zuletzt gemeldete Punkte und Zeitpunkt der letzten GT911-Meldung
    stand: Points,
    letzte_meldung: i64,
    pub addr: u8,
    pub interrupt: bool,
}

// SAFETY: Der Treiber wird nach dem Start nur noch vom Touch-Thread benutzt.
unsafe impl Send for Touch {}

impl Touch {
    pub fn new(bus: &Bus) -> Result<Self, EspError> {
        let Some(addr) = ADDR_TOUCH.into_iter().find(|&a| bus.probe(a)) else {
            return Err(EspError::from_infallible::<{ sys::ESP_ERR_NOT_FOUND as i32 }>());
        };
        // Entspricht ESP_LCD_TOUCH_IO_I2C_GT911_CONFIG() mit Adresse und Takt
        // des Busses (wie im BSP).
        let mut io_cfg: sys::esp_lcd_panel_io_i2c_config_t = zeroed();
        io_cfg.dev_addr = addr as u32;
        io_cfg.scl_speed_hz = I2C_HZ;
        io_cfg.control_phase_bytes = 1;
        io_cfg.dc_bit_offset = 0;
        io_cfg.lcd_cmd_bits = 16;
        io_cfg.flags.set_disable_control_phase(1);
        let mut io = core::ptr::null_mut();
        // SAFETY: gültige Konfiguration, Bus lebt so lange wie das Programm.
        esp!(unsafe { sys::esp_lcd_new_panel_io_i2c_v2(bus.handle(), &io_cfg, &mut io) })?;

        let mut gt911: sys::esp_lcd_touch_io_gt911_config_t = zeroed();
        gt911.dev_addr = addr;
        let mut cfg: sys::esp_lcd_touch_config_t = zeroed();
        cfg.x_max = LCD_W as u16;
        cfg.y_max = LCD_H as u16;
        cfg.rst_gpio_num = sys::gpio_num_t_GPIO_NUM_NC;
        // Pegel wie im BSP (`levels = { .reset = 0, .interrupt = 0 }`, also
        // die Nullwerte von `zeroed`).
        cfg.int_gpio_num = sys::gpio_num_t_GPIO_NUM_NC;
        if let Some(pin) = TOUCH_INT {
            // Der Treiber hängt sich an den GPIO-Interruptdienst.
            // SAFETY: einmalige Einrichtung; „schon installiert“ ist kein Fehler.
            unsafe { sys::gpio_install_isr_service(0) };
            cfg.int_gpio_num = pin;
            cfg.interrupt_callback = Some(on_touch_int);
        }
        cfg.driver_data = (&mut gt911 as *mut sys::esp_lcd_touch_io_gt911_config_t).cast();
        let mut tp = core::ptr::null_mut();
        // SAFETY: wie oben; `gt911` wird nur während des Aufrufs gelesen.
        esp!(unsafe { sys::esp_lcd_touch_new_i2c_gt911(io, &cfg, &mut tp) })?;
        let t = Self { _tp: tp, io, roh_alt: Vec::new(), treiber_alt: [None; MAX_TOUCH], stand: [None; MAX_TOUCH], letzte_meldung: 0, addr, interrupt: TOUCH_INT.is_some() };
        if ROH {
            t.konfig_protokollieren();
        }
        Ok(t)
    }

    /// GT911-Register lesen (16-Bit-Adresse wie im Treiber).
    fn reg(&self, reg: u16, buf: &mut [u8]) -> bool {
        // SAFETY: gültiger Kanal, Puffer `buf.len()` Byte.
        unsafe { sys::esp_lcd_panel_io_rx_param(self.io, reg as i32, buf.as_mut_ptr().cast(), buf.len()) == sys::ESP_OK }
    }

    /// Diagnose V1: Kennung, Firmware, Auflösung und die Konfiguration
    /// (0x8047–0x80FF) einmal ins Log.
    fn konfig_protokollieren(&self) {
        let mut id = [0u8; 11];
        if self.reg(0x8140, &mut id) {
            log::warn!(
                "GT911 roh: Produkt {:?}, Firmware {:02x}{:02x}, Auflösung {}x{}, Hersteller {:02x}",
                core::str::from_utf8(&id[..4]).unwrap_or("?"),
                id[5], id[4],
                u16::from_le_bytes([id[6], id[7]]),
                u16::from_le_bytes([id[8], id[9]]),
                id[10]
            );
        }
        let mut k = [0u8; 186];
        for (i, teil) in k.chunks_mut(32).enumerate() {
            let a = 0x8047 + (i * 32) as u16;
            if !self.reg(a, teil) {
                log::warn!("GT911 roh: Konfiguration ab {a:04x} nicht lesbar");
            }
        }
        for (i, z) in k.chunks(16).enumerate() {
            let hex: Vec<String> = z.iter().map(|b| format!("{b:02x}")).collect();
            log::warn!("GT911 roh: Konfig {:04x}: {}", 0x8047 + i * 16, hex.join(" "));
        }
    }

    /// Diagnose V1: Status 0x814E und Punktdaten bei jeder Änderung ins Log
    /// (vor dem Treiber gelesen; Lesen ändert den Status nicht).
    fn roh_protokollieren(&mut self) {
        let mut b = [0u8; 41];
        if !self.reg(0x814E, &mut b) {
            log::warn!("GT911 roh: Lesefehler");
            return;
        }
        let n = (b[0] & 0x0F) as usize;
        let stand: Vec<u8> = if b[0] & 0x80 != 0 { b[..1 + n.min(5) * 8].to_vec() } else { vec![b[0]] };
        if stand != self.roh_alt {
            let mut t = format!("GT911 roh: {} ms Status {:02x} (bereit {}, Punkte {}, Taste {})", hw::now_us() / 1000, b[0], b[0] >> 7, n, b[0] >> 4 & 1);
            if b[0] & 0x80 != 0 {
                for i in 0..n.min(5) {
                    let p = &b[1 + i * 8..9 + i * 8];
                    t += &format!(
                        " | id {} x {} y {} grösse {}",
                        p[0],
                        u16::from_le_bytes([p[1], p[2]]),
                        u16::from_le_bytes([p[3], p[4]]),
                        u16::from_le_bytes([p[5], p[6]])
                    );
                }
            }
            log::warn!("{t}");
            self.roh_alt = stand;
        }
    }

    /// Touches in panel coordinates (portrait).
    pub fn read(&mut self) -> Points {
        if ROH {
            self.roh_protokollieren();
        }
        // Eigene Abfrage statt esp_lcd_touch_read_data (V1, Auftrag 13.4):
        // Der Treiber schreibt auch dann 0 ins Statusregister, wenn keine
        // neuen Daten bereit sind – meldet der GT911 genau dazwischen, ist
        // die Meldung weg. War es die „losgelassen“-Meldung, behielt der
        // Treiber den letzten Punkt für immer (Ursache des klemmenden Punkts).
        // Hier: quittiert wird nur, was gelesen wurde.
        let mut b = [0u8; 41];
        let jetzt = hw::now_us();
        if !self.reg(0x814E, &mut b[..1]) {
            return self.stand;
        }
        if b[0] & 0x80 != 0 {
            let n = (b[0] & 0x0F) as usize;
            let gueltig = n <= 5 && (n == 0 || self.reg(0x814F, &mut b[1..1 + n * 8]));
            self.reg_schreiben(0x814E, 0);
            if gueltig {
                let mut neu = [None; MAX_TOUCH];
                for (slot, i) in neu.iter_mut().zip(0..n) {
                    let p = &b[1 + i * 8..9 + i * 8];
                    let x = u16::from_le_bytes([p[1], p[2]]).min(LCD_W as u16 - 1);
                    let y = u16::from_le_bytes([p[3], p[4]]).min(LCD_H as u16 - 1);
                    *slot = Some((x, y));
                }
                self.stand = neu;
                self.letzte_meldung = jetzt;
            }
        } else if self.stand.iter().any(Option::is_some) && jetzt - self.letzte_meldung > LOSGELASSEN_US {
            // Solange ein Finger liegt, meldet der GT911 in jedem Messzyklus
            // (ca. 10 ms). Bleibt das aus, ist der Finger weg – auch wenn die
            // Meldung dazu verloren ging.
            if ROH {
                log::warn!("GT911: {} ms keine Meldung bei gesetztem Punkt – losgelassen", LOSGELASSEN_US / 1000);
            }
            self.stand = [None; MAX_TOUCH];
        }
        if ROH && self.stand != self.treiber_alt {
            log::warn!("GT911 gemeldet: {:?}", self.stand);
            self.treiber_alt = self.stand;
        }
        self.stand
    }

    /// GT911-Register schreiben (ein Byte).
    fn reg_schreiben(&self, reg: u16, wert: u8) -> bool {
        let w = [wert];
        // SAFETY: gültiger Kanal, ein Byte.
        unsafe { sys::esp_lcd_panel_io_tx_param(self.io, reg as i32, w.as_ptr().cast(), 1) == sys::ESP_OK }
    }
}

#[derive(Default)]
struct Shared {
    now: Points,
    /// Finger, die seit dem letzten Abholen aufgesetzt haben.
    pressed: Points,
    reads: u32,
    watch_changes: u32,
}

/// Touch im eigenen Thread (Kern 0, über der Spielschleife).
pub struct TouchReader {
    shared: Arc<Mutex<Shared>>,
    pub addr: u8,
}

impl TouchReader {
    /// `watch`: GPIO, dessen Pegelwechsel mitgezählt werden (nur `hwtest`,
    /// um herauszufinden, ob der Touch-Interrupt angeschlossen ist).
    pub fn start(mut touch: Touch, watch: Option<i32>) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let addr = touch.addr;
        let s = shared.clone();
        if let Some(pin) = watch {
            let _ = hw::input_pullup(&[pin]);
        }
        spawn_pinned(c"touch", 0, 10, 6 * 1024, move || {
            let mut last_level = watch.map(hw::is_low);
            let mut since_read = 0;
            loop {
                if touch.interrupt {
                    // Auf den Interrupt warten (Sicherheitsnetz: spätestens nach 20 ms).
                    // SAFETY: 1 ms schlafen.
                    unsafe { sys::vTaskDelay(1) };
                    since_read += 1;
                    if !INT_SEEN.swap(false, Ordering::AcqRel) && since_read < 20 {
                        continue;
                    }
                } else {
                    // SAFETY: schlafen bis zur nächsten Abfrage.
                    unsafe { sys::vTaskDelay(TOUCH_POLL_MS) };
                }
                since_read = 0;
                let pts = touch.read();
                let changed = watch.zip(last_level).map(|(pin, old)| {
                    let now = hw::is_low(pin);
                    last_level = Some(now);
                    now != old
                });
                let Ok(mut sh) = s.lock() else { return };
                let sh = &mut *sh;
                for ((p, now), pressed) in pts.iter().zip(&sh.now).zip(&mut sh.pressed) {
                    if p.is_some() && now.is_none() {
                        *pressed = *p;
                    }
                }
                sh.now = pts;
                sh.reads = sh.reads.wrapping_add(1);
                if changed == Some(true) {
                    sh.watch_changes = sh.watch_changes.wrapping_add(1);
                }
            }
        });
        Self { shared, addr }
    }

    /// Finger für dieses Bild: der aktuelle Stand; ein Finger, der seit dem
    /// letzten Abruf aufgesetzt und schon wieder losgelassen hat, wird einmal
    /// nachgereicht (kurzes Antippen geht nicht verloren).
    pub fn take(&self) -> Points {
        let Ok(mut sh) = self.shared.lock() else { return [None; MAX_TOUCH] };
        let mut out = sh.now;
        for (o, p) in out.iter_mut().zip(sh.pressed.iter_mut()) {
            if o.is_none() {
                *o = *p;
            }
            *p = None;
        }
        out
    }

    /// Zähler seit dem Start: (Abfragen, Pegelwechsel am beobachteten Pin).
    pub fn counters(&self) -> (u32, u32) {
        self.shared.lock().map(|s| (s.reads, s.watch_changes)).unwrap_or((0, 0))
    }
}
