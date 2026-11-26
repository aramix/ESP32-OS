//! Kleine Helfer rund um die ESP-IDF-C-Schnittstellen: GPIO, Zeit, Threads
//! auf einem bestimmten Kern, ausgerichtete Puffer.

use core::ffi::CStr;
use esp_idf_svc::sys::{self, EspError, esp};

/// Leere C-Konfigurationsstruktur (alle Felder 0 = Treiber-Standard).
///
/// Nur für reine Datenstrukturen aus den ESP-IDF-Bindungen gedacht, für die
/// „alles 0“ ein gültiger Wert ist (Zahlen, Zeiger, Bitfelder).
pub fn zeroed<T>() -> T {
    // SAFETY: siehe oben – C-Konfigurationsstrukturen ohne Rust-Invarianten.
    unsafe { core::mem::zeroed() }
}

/// Mikrosekunden seit dem Start.
pub fn now_us() -> i64 {
    // SAFETY: reine Abfrage des Systemzeitgebers.
    unsafe { sys::esp_timer_get_time() }
}

pub fn delay_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

/// Eingänge mit internem Pull-up.
pub fn input_pullup(pins: &[i32]) -> Result<(), EspError> {
    let mut cfg: sys::gpio_config_t = zeroed();
    cfg.pin_bit_mask = pins.iter().fold(0u64, |m, &p| m | (1u64 << p));
    cfg.mode = sys::gpio_mode_t_GPIO_MODE_INPUT;
    cfg.pull_up_en = sys::gpio_pullup_t_GPIO_PULLUP_ENABLE;
    // SAFETY: gültige Konfiguration, Pins existieren auf dem ESP32-P4.
    esp!(unsafe { sys::gpio_config(&cfg) })
}

/// Ausgang mit Startpegel.
pub fn output(pin: i32, high: bool) -> Result<(), EspError> {
    let mut cfg: sys::gpio_config_t = zeroed();
    cfg.pin_bit_mask = 1u64 << pin;
    cfg.mode = sys::gpio_mode_t_GPIO_MODE_OUTPUT;
    // SAFETY: wie oben.
    esp!(unsafe { sys::gpio_config(&cfg) })?;
    set(pin, high);
    Ok(())
}

pub fn set(pin: i32, high: bool) {
    // SAFETY: Pin wurde vorher als Ausgang eingerichtet.
    unsafe { sys::gpio_set_level(pin, high as u32) };
}

/// Pin liegt auf GND (Knopf gedrückt).
pub fn is_low(pin: i32) -> bool {
    // SAFETY: reines Lesen eines Eingangs.
    unsafe { sys::gpio_get_level(pin) == 0 }
}

/// Wie [`spawn_pinned`], aber mit dem Stapel im PSRAM (für große Stapel,
/// z. B. Quake: einige Funktionen legen über 100 KB lokal an).
pub fn spawn_pinned_psram(
    name: &'static CStr,
    core: i32,
    prio: usize,
    stack: usize,
    f: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    // SAFETY: wie in spawn_pinned; Stapel aus dem PSRAM (8-Bit-zugreifbar).
    unsafe {
        let mut cfg = sys::esp_pthread_get_default_config();
        cfg.stack_size = stack as _;
        cfg.prio = prio as _;
        cfg.pin_to_core = core as _;
        cfg.thread_name = name.as_ptr();
        cfg.inherit_cfg = false;
        cfg.stack_alloc_caps = sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT;
        sys::esp_pthread_set_cfg(&cfg);
    }
    let handle = std::thread::Builder::new().stack_size(stack).spawn(f).expect("Thread starten");
    // SAFETY: zurück auf die Standardwerte für spätere Threads.
    unsafe {
        let cfg = sys::esp_pthread_get_default_config();
        sys::esp_pthread_set_cfg(&cfg);
    }
    handle
}

/// Startet einen Thread fest auf Kern `core` mit FreeRTOS-Priorität `prio`.
pub fn spawn_pinned(
    name: &'static CStr,
    core: i32,
    prio: usize,
    stack: usize,
    f: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    // SAFETY: Die pthread-Konfiguration gilt nur für den nächsten Thread, den
    // dieser Thread startet; `name` lebt für immer.
    unsafe {
        let mut cfg = sys::esp_pthread_get_default_config();
        cfg.stack_size = stack as _;
        cfg.prio = prio as _;
        cfg.pin_to_core = core as _;
        cfg.thread_name = name.as_ptr();
        cfg.inherit_cfg = false;
        sys::esp_pthread_set_cfg(&cfg);
    }
    let handle = std::thread::Builder::new().stack_size(stack).spawn(f).expect("Thread starten");
    // SAFETY: zurück auf die Standardwerte für spätere Threads.
    unsafe {
        let cfg = sys::esp_pthread_get_default_config();
        sys::esp_pthread_set_cfg(&cfg);
    }
    handle
}

/// Set up logging: our own logger writes each line into a ring buffer through
/// the console driver ([`crate::serial`]); an interrupt sends it.
/// Der Weg über stdout schrieb blockierend mit 115200 Baud – ~9 ms je
/// Zeile; die Statistik alle 2 s hielt die Spielschleife so ~70 ms an, ein
/// sichtbarer Hänger. Ohne Treiber bleibt es beim Logger von ESP-IDF.
pub fn log_einrichten() {
    match crate::serial::init() {
        Ok(()) => {
            static LOGGER: SerialLog = SerialLog;
            if log::set_logger(&LOGGER).is_ok() {
                log::set_max_level(log::LevelFilter::Info);
            }
            stdout_umleiten();
        }
        Err(e) => {
            esp_idf_svc::log::EspLogger::initialize_default();
            log::error!("console driver: {e} - logging blocks");
        }
    }
}

/// C-Ausgabe (printf von Doom, Quake und ESP-IDF) ebenfalls über den
/// Ringpuffer: Über VFS gab stdout jedes Zeichen einzeln an den Treiber –
/// mit je ~16 Byte Verwaltung im Ringpuffer, der so nach ~250 Zeichen voll
/// war und dann im Takt von 115200 Baud blockierte. Jetzt ein eigener,
/// zeilengepufferter Strom: eine ganze Zeile je Aufruf.
fn stdout_umleiten() {
    unsafe extern "C" fn schreiben(_k: *mut core::ffi::c_void, buf: *const core::ffi::c_char, n: core::ffi::c_int) -> core::ffi::c_int {
        if n > 0 && !LOG_STUMM.load(core::sync::atomic::Ordering::Acquire) {
            // newlib: `buf` is valid for `n` bytes
            crate::serial::write(buf.cast(), n as usize);
        }
        n
    }
    // SAFETY: einmal beim Start, bevor weitere Tasks entstehen (sie erben
    // stdout von der globalen Reentrant-Struktur).
    unsafe {
        let f = sys::funopen(core::ptr::null(), None, Some(schreiben), None, None);
        if f.is_null() {
            log::error!("stdout: funopen fehlgeschlagen – C-Ausgabe blockiert weiter");
            return;
        }
        sys::setvbuf(f, core::ptr::null_mut(), sys::_IOLBF as _, 256);
        let r = sys::__getreent();
        (*r)._stdout = f;
        (*r)._stderr = f;
        (*sys::_global_impure_ptr)._stdout = f;
        (*sys::_global_impure_ptr)._stderr = f;
    }
}

/// Logausgabe kurz aussetzen (Bildschirmfoto über dieselbe Leitung).
pub static LOG_STUMM: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Zeilen wie der ESP-IDF-Logger: `I (Millisekunden) Ziel: Text`.
struct SerialLog;

impl log::Log for SerialLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::max_level()
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) || LOG_STUMM.load(core::sync::atomic::Ordering::Acquire) {
            return;
        }
        use std::fmt::Write;
        let k = match r.level() {
            log::Level::Error => 'E',
            log::Level::Warn => 'W',
            log::Level::Info => 'I',
            log::Level::Debug => 'D',
            log::Level::Trace => 'V',
        };
        let mut z = String::with_capacity(160);
        // SAFETY: reine Abfrage der Laufzeit.
        let ms = unsafe { sys::esp_log_timestamp() };
        let _ = writeln!(z, "{k} ({ms}) {}: {}", r.target(), r.args());
        crate::serial::write(z.as_bytes().as_ptr(), z.len());
        // Send errors (including panics) at once; a restart may follow.
        if r.level() == log::Level::Error {
            crate::serial::flush();
        }
    }

    fn flush(&self) {}
}

/// Interner RAM, den die Bildpuffer frei lassen (Bytes). Das OS-Programm
/// setzt hier eine Reserve für das WLAN (esp_hosted, lwIP, Ereignis-
/// Aufgaben): Ohne sie nahm die Pipeline je nach Startreihenfolge fast den
/// ganzen internen RAM, und der WLAN-Start blieb hängen (W1).
pub static INTERN_RESERVE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// M2: Bildstreifen dürfen in den internen RAM (false im Desktop-Modus).
pub static PIXEL_INTERN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// Pixelpuffer (u16) für immer, 128-Byte-ausgerichtet (Cache-Zeile, DMA).
/// `internal`: im schnellen internen RAM (nur, wenn danach noch
/// [`INTERN_RESERVE`] frei bleibt), sonst im PSRAM.
pub fn alloc_pixels(n: usize, internal: bool) -> Option<&'static mut [u16]> {
    // M2: im Desktop-Modus keine Bildstreifen im internen RAM (Messung M1:
    // 126 KB, intern nur noch 86 KB frei) – der Desktop zeichnet wenig neu;
    // Spiele starten neu in ihren eigenen Modus und behalten die schnellen
    // internen Streifen
    if internal && !PIXEL_INTERN.load(core::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    if internal {
        let reserve = INTERN_RESERVE.load(core::sync::atomic::Ordering::Relaxed);
        // SAFETY: reine Abfrage.
        let frei = unsafe { sys::heap_caps_get_free_size(sys::MALLOC_CAP_INTERNAL) };
        if reserve > 0 && frei < n * 2 + reserve {
            return None;
        }
    }
    let caps = if internal {
        sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT | sys::MALLOC_CAP_DMA
    } else {
        sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT
    };
    // SAFETY: Ergebnis wird auf null geprüft; der Speicher ist genullt und
    // wird nie freigegeben (lebt so lange wie das Programm).
    unsafe {
        let p = sys::heap_caps_aligned_calloc(128, n, 2, caps) as *mut u16;
        (!p.is_null()).then(|| core::slice::from_raw_parts_mut(p, n))
    }
}

/// Chip-Revision (z. B. 300 = v3.0), Kerne, CPU-Takt in MHz, PSRAM in MB.
pub fn chip() -> (u16, u8, u32, usize) {
    let mut info: sys::esp_chip_info_t = zeroed();
    // SAFETY: reine Abfragen.
    unsafe {
        sys::esp_chip_info(&mut info);
        (info.revision, info.cores, sys::esp_rom_get_cpu_ticks_per_us(), sys::esp_psram_get_size() / (1024 * 1024))
    }
}

/// Freier Speicher (intern, PSRAM) in KB – für die Startmeldung.
pub fn free_kb() -> (usize, usize) {
    // SAFETY: reine Abfrage.
    unsafe {
        (
            sys::heap_caps_get_free_size(sys::MALLOC_CAP_INTERNAL) / 1024,
            sys::heap_caps_get_free_size(sys::MALLOC_CAP_SPIRAM) / 1024,
        )
    }
}

/// Zyklenzähler der CPU (RISC-V `cycle`, wie `esp_cpu_get_cycle_count`).
pub fn cycles() -> u32 {
    let c: u32;
    // SAFETY: liest nur den Zählerstand.
    unsafe { core::arch::asm!("csrr {0}, cycle", out(reg) c) };
    c
}

/// Flags für `esp_cache_msync` (in `esp_mm/include/esp_cache.h` von ESP-IDF
/// v5.5.5 als `BIT(n)`-Makros definiert, die bindgen nicht auflöst).
pub const CACHE_MSYNC_UNALIGNED: i32 = 1 << 1;
/// Richtung Cache → Speicher (Zurückschreiben).
pub const CACHE_MSYNC_DIR_C2M: i32 = 1 << 2;
/// Richtung Speicher → Cache (veraltete Zeilen verwerfen).
pub const CACHE_MSYNC_DIR_M2C: i32 = 1 << 3;
