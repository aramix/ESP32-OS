//! Portrait display on MIPI-DSI (2 lanes), video mode; for the panel see
//! [`crate::board`].
//!
//! Der DSI-Controller liest das Bild selbstständig aus einem Bildspeicher im
//! PSRAM (bei den Waveshare-Zeitwerten rechnerisch 55-mal pro Sekunde; der
//! Zähler [`refreshes`] misst den echten Wert). Drei Bildspeicher im Wechsel:
//!
//! * [`Display::acquire`] liefert den ältesten, sobald das Display ihn nicht
//!   mehr ausliest – dorthin schreibt der PPA das neue Bild (streifenweise).
//! * [`Display::present`] schaltet um; wirksam wird das mit dem nächsten
//!   Bildanfang (VSync) – kein Zerreißen. Damit ist die Bildrate auch auf die
//!   Bildwiederholrate des Panels begrenzt.

use crate::board::*;
use crate::hw::zeroed;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use esp_idf_svc::sys::{self, EspError, esp};

pub const FB_COUNT: usize = 3;
pub const FB_BYTES: usize = LCD_W * LCD_H * 2;

/// Fertig ausgelesene Bilder seit dem Start (Interrupt „refresh done“).
static REFRESHES: AtomicU32 = AtomicU32::new(0);

/// Zähler der Bildwechsel des Panels (zum Messen der Wiederholrate).
pub fn refreshes() -> u32 {
    REFRESHES.load(Ordering::Acquire)
}

/// Task, die auf den nächsten Bildwechsel wartet (sonst null).
static WARTET: AtomicPtr<sys::tskTaskControlBlock> = AtomicPtr::new(core::ptr::null_mut());

unsafe extern "C" fn on_refresh_done(
    _panel: sys::esp_lcd_panel_handle_t,
    _edata: *mut sys::esp_lcd_dpi_panel_event_data_t,
    _ctx: *mut c_void,
) -> bool {
    REFRESHES.fetch_add(1, Ordering::Release);
    // Wartende Task sofort wecken (statt 1-ms-Abfrage).
    let t = WARTET.swap(core::ptr::null_mut(), Ordering::AcqRel);
    let mut geweckt: sys::BaseType_t = 0;
    if !t.is_null() {
        // SAFETY: gültiger Task-Handle (die Task wartet gerade); aus der ISR.
        unsafe { sys::vTaskGenericNotifyGiveFromISR(t, 0, &mut geweckt) };
    }
    geweckt != 0
}

/// Wartet, bis `fertig()` gilt – geweckt vom Bildwechsel-Interrupt.
fn warte_auf_bildwechsel(fertig: impl Fn() -> bool) {
    while !fertig() {
        // SAFETY: eigener Task-Handle; die Benachrichtigung zählt, auch wenn
        // der Interrupt zwischen Eintragen und Warten kommt. Höchstens 2 ms
        // warten (Sicherheitsnetz, falls ein Interrupt verloren ginge).
        unsafe {
            WARTET.store(sys::xTaskGetCurrentTaskHandle(), Ordering::Release);
            if fertig() {
                WARTET.store(core::ptr::null_mut(), Ordering::Release);
                break;
            }
            sys::ulTaskGenericNotifyTake(0, 1, 2);
        }
    }
}

pub struct Display {
    panel: sys::esp_lcd_panel_handle_t,
    /// DBI-Befehlskanal (DCS-Befehle wie Sleep-In, die der HX8394-Treiber
    /// nicht anbietet).
    io: sys::esp_lcd_panel_io_handle_t,
    fbs: [*mut u16; FB_COUNT],
    /// Angezeigter (bzw. ab dem nächsten Bild angezeigter) Bildspeicher.
    shown: usize,
    /// Bildspeicher, in den gerade geschrieben wird.
    target: usize,
    /// `REFRESHES` beim letzten und vorletzten Umschalten.
    last_switch: u32,
    prev_switch: u32,
    brightness: u8,
}

impl Display {
    pub fn new(fill: u16) -> Result<Self, EspError> {
        // SAFETY: Reihenfolge und Werte wie im Board-Support-Paket von
        // Waveshare; Konfigurationen leben bis zum Ende der Aufrufe, Handles
        // bis zum Programmende.
        unsafe {
            // 1) Versorgung der MIPI-D-PHY (LDO-Kanal 3, 2,5 V).
            let mut ldo_cfg: sys::esp_ldo_channel_config_t = zeroed();
            ldo_cfg.chan_id = DSI_PHY_LDO_CHAN;
            ldo_cfg.voltage_mv = DSI_PHY_LDO_MV;
            let mut ldo = core::ptr::null_mut();
            esp!(sys::esp_ldo_acquire_channel(&ldo_cfg, &mut ldo))?;

            // 2) DSI-Bus und Befehlskanal (DBI) für die Initialisierung.
            let mut bus_cfg: sys::esp_lcd_dsi_bus_config_t = zeroed();
            bus_cfg.bus_id = 0;
            bus_cfg.num_data_lanes = DSI_LANES;
            bus_cfg.lane_bit_rate_mbps = DSI_LANE_MBPS as _; // ESP-IDF 5.5: f32
            let mut bus = core::ptr::null_mut();
            esp!(sys::esp_lcd_new_dsi_bus(&bus_cfg, &mut bus))?;
            let mut io_cfg: sys::esp_lcd_dbi_io_config_t = zeroed();
            io_cfg.virtual_channel = 0;
            io_cfg.lcd_cmd_bits = 8;
            io_cfg.lcd_param_bits = 8;
            let mut io = core::ptr::null_mut();
            esp!(sys::esp_lcd_new_panel_io_dbi(bus, &io_cfg, &mut io))?;

            // 3) Videotakt und Bildspeicher (DPI), RGB565, drei Puffer.
            let mut dpi: sys::esp_lcd_dpi_panel_config_t = zeroed();
            dpi.virtual_channel = 0;
            dpi.dpi_clk_src = sys::soc_periph_mipi_dsi_dpi_clk_src_t_MIPI_DSI_DPI_CLK_SRC_DEFAULT;
            dpi.dpi_clock_freq_mhz = DPI_CLOCK_MHZ as _;
            dpi.pixel_format = sys::lcd_color_rgb_pixel_format_t_LCD_COLOR_PIXEL_FORMAT_RGB565;
            dpi.num_fbs = FB_COUNT as u8;
            dpi.video_timing.h_size = LCD_W as u32;
            dpi.video_timing.v_size = LCD_H as u32;
            dpi.video_timing.hsync_pulse_width = HSYNC_PULSE;
            dpi.video_timing.hsync_back_porch = HSYNC_BACK;
            dpi.video_timing.hsync_front_porch = HSYNC_FRONT;
            dpi.video_timing.vsync_pulse_width = VSYNC_PULSE;
            dpi.video_timing.vsync_back_porch = VSYNC_BACK;
            dpi.video_timing.vsync_front_porch = VSYNC_FRONT;

            // 4) Panel driver (sends the init sequence).
            let panel = create_panel(bus, io, &dpi)?;
            esp!(sys::esp_lcd_panel_reset(panel))?;
            esp!(sys::esp_lcd_panel_init(panel))?;
            // Nicht jeder Treiber kennt An/Aus – ein Fehler hier ist egal.
            let _ = sys::esp_lcd_panel_disp_on_off(panel, true);

            // 5) Bildspeicher holen und mit der Hintergrundfarbe füllen.
            let mut fb: [*mut c_void; FB_COUNT] = [core::ptr::null_mut(); FB_COUNT];
            esp!(sys::esp_lcd_dpi_panel_get_frame_buffer(
                panel,
                FB_COUNT as u32,
                &mut fb[0] as *mut *mut c_void,
                &mut fb[1] as *mut *mut c_void,
                &mut fb[2] as *mut *mut c_void,
            ))?;
            let fbs = fb.map(|p| p as *mut u16);
            for &p in &fbs {
                core::slice::from_raw_parts_mut(p, LCD_W * LCD_H).fill(fill);
                esp!(sys::esp_cache_msync(p.cast(), FB_BYTES, crate::hw::CACHE_MSYNC_DIR_C2M as _))?;
            }
            // ESP-IDF 5.5: `on_refresh_done` ist veraltet und teilt sich eine
            // Union mit dem Nachfolger `on_frame_buf_complete` (gleiche
            // Bedeutung: Bildspeicher wird nicht mehr gelesen).
            let cbs = sys::esp_lcd_dpi_panel_event_callbacks_t {
                on_color_trans_done: None,
                __bindgen_anon_1: sys::esp_lcd_dpi_panel_event_callbacks_t__bindgen_ty_1 {
                    on_frame_buf_complete: Some(on_refresh_done),
                },
                on_vsync: None,
            };
            esp!(sys::esp_lcd_dpi_panel_register_event_callbacks(panel, &cbs, core::ptr::null_mut()))?;

            let mut d = Self { panel, io, fbs, shown: 0, target: 0, last_switch: 0, prev_switch: 0, brightness: 0 };
            d.backlight_init()?;
            Ok(d)
        }
    }

    /// Hintergrundlicht per PWM (LEDC, 5 kHz, 10 Bit, Timer 1, Kanal 1) –
    /// wie im Board-Support-Paket.
    fn backlight_init(&mut self) -> Result<(), EspError> {
        let mut timer: sys::ledc_timer_config_t = zeroed();
        timer.speed_mode = sys::ledc_mode_t_LEDC_LOW_SPEED_MODE;
        timer.duty_resolution = sys::ledc_timer_bit_t_LEDC_TIMER_10_BIT;
        timer.timer_num = sys::ledc_timer_t_LEDC_TIMER_1;
        timer.freq_hz = 5000;
        let mut ch: sys::ledc_channel_config_t = zeroed();
        ch.gpio_num = LCD_BACKLIGHT;
        ch.speed_mode = sys::ledc_mode_t_LEDC_LOW_SPEED_MODE;
        ch.channel = sys::ledc_channel_t_LEDC_CHANNEL_1;
        ch.timer_sel = sys::ledc_timer_t_LEDC_TIMER_1;
        ch.duty = 0;
        // SAFETY: gültige Konfigurationen.
        unsafe {
            esp!(sys::ledc_timer_config(&timer))?;
            esp!(sys::ledc_channel_config(&ch))?;
        }
        Ok(())
    }

    /// Helligkeit 1..=10 (quadratisch, damit die Stufen gleichmäßig wirken).
    /// Helligkeit 1..=10; 0 = Hintergrundlicht aus (Bildschirmschoner).
    pub fn set_brightness(&mut self, level: u8) {
        let level = level.min(10);
        if level == self.brightness {
            return;
        }
        self.brightness = level;
        let duty = if level == 0 { 0 } else { (1023 * level as u32 * level as u32 / 100).max(40) };
        // SAFETY: Kanal wurde in `backlight_init` eingerichtet.
        unsafe {
            sys::ledc_set_duty(sys::ledc_mode_t_LEDC_LOW_SPEED_MODE, sys::ledc_channel_t_LEDC_CHANNEL_1, duty);
            sys::ledc_update_duty(sys::ledc_mode_t_LEDC_LOW_SPEED_MODE, sys::ledc_channel_t_LEDC_CHANNEL_1);
        }
    }

    /// Nächster Bildspeicher zum Beschreiben. Wartet, bis das Display ihn
    /// nicht mehr ausliest: Er war bis zum vorletzten Umschalten zu sehen und
    /// ist frei, sobald danach ein Bild fertig ausgelesen wurde.
    /// Panel schlafen legen (DCS Display-Off 0x28 + Sleep-In 0x10) bzw.
    /// wecken (Sleep-Out 0x11, 120 ms laut HX8394-Datenblatt, Display-On
    /// 0x29). Der HX8394-Treiber kennt `disp_sleep` nicht (0x106), daher
    /// direkt über den DBI-Kanal. Ergebnis: ESP-Fehlercodes (0 = ok).
    pub fn schlafen(&mut self, an: bool) -> (i32, i32) {
        // SAFETY: gültiger DBI-Kanal aus `new`, Befehle ohne Parameter.
        unsafe {
            let dcs = |c: i32| sys::esp_lcd_panel_io_tx_param(self.io, c, core::ptr::null(), 0);
            if an {
                let a = dcs(0x28);
                std::thread::sleep(std::time::Duration::from_millis(20));
                let b = dcs(0x10);
                (a, b)
            } else {
                let b = dcs(0x11);
                std::thread::sleep(std::time::Duration::from_millis(120));
                let a = dcs(0x29);
                (a, b)
            }
        }
    }

    pub fn acquire(&mut self) -> *mut u16 {
        self.target = (self.shown + 1) % FB_COUNT;
        let prev = self.prev_switch;
        warte_auf_bildwechsel(|| refreshes().wrapping_sub(prev) != 0);
        self.fbs[self.target]
    }

    /// Zeigt den mit [`Display::acquire`] geholten Bildspeicher ab dem
    /// nächsten Bildanfang an. Liefert den Bildwechsel-Zähler, ab dem er zu
    /// sehen ist (für gleichmäßigen Spieltakt, siehe `FramePacer`).
    /// Wie [`Display::present`], aber frühestens `n` Bildwechsel nach dem
    /// vorigen Bild (gleichmäßig halbe Bildrate bei `n` = 2).
    pub fn present_nach(&mut self, n: u32) -> u32 {
        if n > 1 {
            let last = self.last_switch;
            warte_auf_bildwechsel(|| refreshes().wrapping_sub(last) >= n);
        }
        self.present()
    }

    /// Der gerade angezeigte Bildspeicher (für Bildschirmfotos).
    pub fn gezeigt(&self) -> *const u16 {
        self.fbs[self.shown]
    }

    pub fn present(&mut self) -> u32 {
        // SAFETY: Der Zeiger ist ein Bildspeicher des Treibers → er schaltet
        // nur um (kein Kopieren, nur Cache-Abgleich).
        unsafe {
            sys::esp_lcd_panel_draw_bitmap(
                self.panel,
                0,
                0,
                LCD_W as i32,
                LCD_H as i32,
                self.fbs[self.target] as *const c_void,
            );
        }
        self.shown = self.target;
        self.prev_switch = self.last_switch;
        self.last_switch = refreshes();
        // Umgeschaltet wird am Ende des laufenden Bildes.
        self.last_switch.wrapping_add(1)
    }
}
