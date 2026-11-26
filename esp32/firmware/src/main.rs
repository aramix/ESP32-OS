//! Spielekonsole (Pudding-Panik, Doom, Würfelwelt) – Firmware für das
//! Waveshare ESP32-P4-WIFI6-Touch-LCD-5.
//!
//! * **Start:** Das Startziel steht im NVS (gesetzt vor einem Neustart, beim
//!   Lesen gelöscht) – ohne Ziel startet das Konsolen-Menü. Jedes Spiel
//!   läuft so mit frischem Speicher; Spielwechsel = Ziel merken + Neustart.
//!
//! * **Kern 0:** Knöpfe und Touch lesen, Spiel simulieren (fest 120 Schritte
//!   pro Sekunde, angezeigt wird zwischen den Schritten interpoliert), die
//!   geraden Bildstreifen rendern, Spielstand speichern.
//! * **Kern 1:** gleichzeitig die ungeraden Bildstreifen und – mit höherer
//!   Priorität – den Ton (Synthesizer → I²S → ES8311).
//! * **PPA:** vergrößert und dreht jeden fertigen Streifen in den
//!   Bildspeicher; umgeschaltet wird mit dem nächsten Bildanfang (VSync).
//! * **Takt:** Die Spielzeit rückt pro Bild um die Zahl der Bildwechsel des
//!   Panels vor (nicht um die schwankende CPU-Zeit) – gleichmäßig bei 55 Hz.
//!
//! Die Spiele (`konsole`, `pudding-core`) sind dieselben, am PC getesteten
//! Bibliotheken wie in der PC-Simulation.

use core::sync::atomic::{AtomicU32, Ordering};
use konsole::start::{self, Speicher};
use konsole::{Rahmen, Spiel, SpielId, Wechsel};
use pudding_core::app::{FrameData, Perf};
use pudding_core::audio::Synth;
use pudding_core::gfx::hex;
use pudding_core::input::{Input, panel_to_raster};
use pudding_core::present::FramePacer;
use pudding_fw::board::{BATTERY, BUTTONS_ENABLED, I2C_PORT, I2C_SCL, I2C_SDA, LCD_H, LCD_W, PANEL_HZ};
use pudding_fw::display::Display;
use pudding_fw::pipeline::{Frame, FrameStats, Pipeline};
use pudding_fw::{audio, battery, codec, hw, i2c, slots, store, touch};
#[cfg(feature = "doom")]
use pudding_fw::doom_board;
#[cfg(feature = "quake")]
use pudding_fw::quake_board;

/// Messlauf (Autopilot, Rundgang): Spielstände und Einstellungen werden
/// nicht gespeichert – Messungen verändern nichts beim Nutzer.
/// ESP32-OS: Programme belegen aus ihrem eigenen Heap (Kern K1).
#[global_allocator]
static ALLOC: kern::KernAlloc = kern::KernAlloc;

/// Autopilot (eingespielte Berührungen, Entwicklung): aus der Datei
/// `ESP32OS_AUTOPILOT_DATEI` (lange Abläufe – eine Umgebungsvariable darf
/// unter Windows höchstens 32767 Zeichen haben) oder `ESP32OS_AUTOPILOT`.
#[cfg(esp32os_autopilot_datei)]
const AUTOPILOT: Option<&str> = Some(include_str!(env!("ESP32OS_AUTOPILOT_PFAD")));
#[cfg(not(esp32os_autopilot_datei))]
const AUTOPILOT: Option<&str> = option_env!("ESP32OS_AUTOPILOT");

const MESSLAUF: bool = option_env!("KONSOLE_AUTOPILOT").is_some() || option_env!("KONSOLE_RUNDGANG").is_some();

fn main() {
    esp_idf_svc::sys::link_patches();
    hw::log_einrichten();
    // Flash-Sperre vor allem anderen (Befund 22.11., flash_sperre.c)
    pudding_fw::flash_sperre::einrichten();
    // Entwicklungs-Build: Belastungstest Flash schreiben + abgebildet lesen
    if let Some(v) = option_env!("ESP32OS_FLASHTEST") {
        pudding_fw::flash_sperre::test(v);
    }
    // Eigene Panikmeldung: Der Standard-Handler liest RUST_BACKTRACE aus der
    // Umgebung – das scheitert hier und verschluckt die eigentliche Meldung.
    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIK: {info}");
    }));
    // ESP32-OS-Kern: Prozesse, Heaps, Aufräumer (Panik in Programmen beendet nur sie).
    // Selbsttests (Entwicklung) laufen vor allem anderen.
    let selbsttest = option_env!("KERN_SELBSTTEST").is_some();
    let dateitest = option_env!("KERN_DATEITEST").is_some();
    kern::sys::debug_ausgabe_setzen(|z| log::info!("[debug] {z}"));
    if selbsttest {
        kern::init(&pudding_fw::kern_board::BOARD, pudding_fw::kern_board::selbsttest::PROGRAMME);
        pudding_fw::kern_board::selbsttest::laufen();
    }
    if dateitest {
        pudding_fw::speicher::einhaengen();
        kern::init(&pudding_fw::kern_board::BOARD, pudding_fw::kern_board::dateitest::PROGRAMME);
        pudding_fw::kern_board::dateitest::laufen();
    }
    pudding_core::render::set_profile_clock(hw::cycles);
    log::info!("ESP32-OS startet …");
    pudding_fw::log_chip();

    log_heap("Start");
    static LOOP_STAGE: AtomicU32 = AtomicU32::new(0);
    static LOOP_FRAMES: AtomicU32 = AtomicU32::new(0);
    hw::spawn_pinned(c"wachhund", 0, 10, 4096, || {
        let mut last = u32::MAX;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            let f = LOOP_FRAMES.load(Ordering::Relaxed);
            if f == last {
                let (i, p) = hw::free_kb();
                log::error!(
                    "HÄNGT bei Bild {f}: Schleife Stufe {}, {}; frei {i} KB intern, {p} KB PSRAM",
                    LOOP_STAGE.load(Ordering::Relaxed),
                    pudding_fw::pipeline::debug_state()
                );
            }
            last = f;
        }
    });


    if let Err(e) = pudding_fw::init_buttons() {
        log::error!("Knöpfe: {e}");
    }

    // --- Startziel und Spielstand -------------------------------------------
    let nvs = store::Nvs::open().inspect_err(|e| log::error!("NVS: {e}")).ok().map(std::sync::Arc::new);
    let ziel = nvs.as_deref().map(|n| start::startziel(n)).unwrap_or(SpielId::Konsole);
    // Entwicklung: `KONSOLE_START=doom` (bzw. pudding, wuerfelwelt) beim Bauen
    // startet ohne Startziel direkt dieses Spiel (Test ohne Antippen).
    let ziel = match (ziel, option_env!("KONSOLE_START")) {
        (SpielId::Konsole, Some("pudding")) => SpielId::Pudding,
        (SpielId::Konsole, Some("doom")) => SpielId::Doom,
        (SpielId::Konsole, Some("wuerfelwelt")) => SpielId::Wuerfelwelt,
        (SpielId::Konsole, Some("quake")) => SpielId::Quake,
        (z, _) => z,
    };
    // Liegt das Spiel in einem anderen Programm (Doom, Quake bzw. zurück zur
    // Konsole)? Dann Startziel wieder hinterlegen, Startpartition umstellen,
    // neu starten.
    log::info!("Programm „{}“, Startziel {ziel:?}", slots::laufend());
    // Entwicklungs-Build für den Rollback-Nachweis (B0): startet nie bis zum
    // Gültig-Melden → der Bootloader springt auf das vorige Programm zurück.
    if option_env!("ESP32OS_DEFEKT").is_some() {
        panic!("ESP32OS_DEFEKT: absichtlich defektes Programm");
    }
    if !slots::hier(ziel) {
        if let Some(n) = nvs.as_deref() {
            n.schreiben(start::START_SCHLUESSEL, &[ziel.als_u8()]);
        }
        if slots::umschalten(ziel) {
            log::info!("{ziel:?} liegt in „{:?}“ – Neustart dorthin", slots::partition_fuer(ziel));
            // SAFETY: Neustart; das Startziel steht im NVS.
            unsafe { esp_idf_svc::sys::esp_restart() };
        }
        if let Some(n) = nvs.as_deref() {
            n.loeschen(start::START_SCHLUESSEL);
        }
    }
    let ziel = if slots::hier(ziel) { ziel } else { SpielId::Konsole };
    // Läuft ein Spielprogramm (Doom, Quake), gleich wieder die Konsole als
    // Startprogramm eintragen: Nach Absturz, Reset oder Ausschalten startet
    // das Board direkt im Menü.
    if !slots::hier(SpielId::Konsole) {
        // vorher gültig melden (Rollback betrifft den eigenen otadata-Eintrag)
        slots::als_gueltig_markieren();
        slots::umschalten(SpielId::Konsole);
    }
    // --- ESP32-OS: Standardstart ist der Desktop ----------------------------
    // Ausnahme: ein Spiel als Startziel (Desktop-Symbol bzw. Ordner „Spiele“).
    // Den alten Launcher gibt es nicht mehr (S0, Auftrag 13.8); ein Merker aus
    // älteren Versionen wird nur gelöscht.
    if let Some(n) = nvs.as_deref()
        && n.lesen(desktop::shell::MODUS_SCHLUESSEL).is_some()
    {
        n.loeschen(desktop::shell::MODUS_SCHLUESSEL);
        log::info!("Alter Merker „Launcher“ gelöscht");
    }
    // Doom-/Quake-Programme starten nie den Desktop (Startziel Konsole →
    // Umschalten ins ESP32-OS-Programm): zur Übersetzungszeit aus, damit
    // Desktop, Kern und WLAN nicht in deren 2,5-MB-Plätze gelinkt werden.
    let desktop_start = !cfg!(any(feature = "doom", feature = "quake"))
        && ziel == SpielId::Konsole
        && !selbsttest
        && !dateitest;
    if desktop_start {
        // B0-Umzug: PAK aus dem alten Flash-Bereich retten, bevor „daten“
        // (neue Lage) eingebunden und ggf. formatiert wird
        // Wert „CRC32:Länge“ der PAK auf dem PC
        if let Some((c, l)) = option_env!("ESP32OS_PAK_RETTEN").and_then(|w| w.split_once(':')).and_then(|(c, l)| Some((u32::from_str_radix(c, 16).ok()?, l.parse::<usize>().ok()?))) {
            pudding_fw::umzug::pak_retten(c, l);
        }
        // Dateisystem (K6): interne FAT-Partition = /sys und /rd/1, microSD = /sd0/1.
        pudding_fw::speicher::einhaengen();
        // Anzeige-/Eingabeschleife über den Programmen (Priorität 3)
        // SAFETY: setzt die Priorität des laufenden Tasks.
        unsafe { esp_idf_svc::sys::vTaskPrioritySet(core::ptr::null_mut(), 5) };
        kern::init(&pudding_fw::kern_board::BOARD, desktop::PROGRAMME);
        // Entwicklungs-Build: Einstellungen setzen („schlüssel=wert;…“)
        if let Some(ini) = option_env!("ESP32OS_INI") {
            for (k, v) in ini.split(';').filter_map(|e| e.split_once('=')) {
                let ok = kolibri::libini::schreiben(desktop::konfig::DATEI, "system", k.trim(), v.trim());
                log::warn!("ESP32OS_INI: {k}={v} ({})", if ok { "gesetzt" } else { "Fehler" });
            }
        }
        // Test-Build V3: SD-Testreste löschen (einmalig, Auftrag 13.6)
        if option_env!("ESP32OS_SD_AUFRAEUMEN").is_some() {
            pudding_fw::speicher::sd_testreste_loeschen();
        }
        // Entwicklungs-Build (Q1 Fund 5): Test-Notizen früherer Board-Läufe
        if option_env!("ESP32OS_TESTNOTIZEN_LOESCHEN").is_some() {
            pudding_fw::speicher::testnotizen_loeschen();
        }
        // Test-Build (A5): Testtöne für den Musikplayer auf die SD-Karte
        #[cfg(esp32os_musik_test)]
        pudding_fw::speicher::musik_testdaten();
        // Test-Build (Z1): Test-PDFs für den PDF-Betrachter auf die SD-Karte
        #[cfg(esp32os_pdf_test)]
        pudding_fw::speicher::pdf_testdaten();
        // Test-Build: Testreste des ersten Browser-Versuchs (B1) löschen
        if option_env!("ESP32OS_NETSURF_RESTE").is_some() {
            pudding_fw::speicher::netsurf_testreste_loeschen();
        }
        // Test-Build V2: Stromausfall beim Speichern der Einstellungen
        if option_env!("ESP32OS_STROMAUSFALL").is_some() {
            pudding_fw::stromtest::lauf(desktop::konfig::DATEI);
        }
        // Entwicklungs-Build: wie ein frisches Gerät (P4-Nachweis)
        if option_env!("ESP32OS_ERSTEINRICHTUNG").is_some() {
            let r = kern::datei::loeschen(desktop::konfig::DATEI);
            log::warn!("ESP32OS_ERSTEINRICHTUNG: Einstellungen gelöscht (Code {r})");
        }
        // Sprache und Tastaturlayout (Auftrag 12.1) vor dem ersten Text laden
        desktop::konfig::laden();
        {
            use core::sync::atomic::Ordering::Relaxed;
            log::info!(
                "Einstellungen: Sprache {}, Helligkeit {}, Bildschirm aus {} s, Ruhe nach {} s, Lautstärke {}, Touchpad {}, Layout {:?}, Hintergrund {:?}, Uhr {}",
                kern::sprache::kuerzel(kern::sprache::aktuell()),
                desktop::konfig::HELLIGKEIT.load(Relaxed),
                desktop::konfig::BILDSCHIRM_AUS.load(Relaxed),
                desktop::konfig::RUHE_NACH.load(Relaxed),
                desktop::konfig::LAUTSTAERKE.load(Relaxed),
                kern::sys::eingabe::touchpad(),
                desktop::tastatur::layout(),
                desktop::konfig::hintergrund(),
                kern::zeit::unix()
            );
        }
        // Autopilot-Builds tippen direkt (Touchpad-Modus nur für diesen Lauf
        // aus, die gespeicherte Einstellung bleibt unverändert)
        if AUTOPILOT.is_some() {
            desktop::konfig::TOUCHPAD_GESPERRT.store(true, core::sync::atomic::Ordering::Relaxed);
            kern::sys::eingabe::touchpad_setzen(false);
            log::warn!("Autopilot: Touchpad-Modus für diesen Lauf aus");
        }
        if option_env!("ESP32OS_KNOPFSUCHE").is_some() {
            pudding_fw::knopfsuche::starten();
        }
        // WLAN über den C6 (Block D) im Hintergrund; die Bildpuffer lassen
        // ihm internen RAM übrig (siehe hw::INTERN_RESERVE)
        hw::INTERN_RESERVE.store(pudding_fw::wlan::INTERN_RESERVE, core::sync::atomic::Ordering::Relaxed);
        // M2: Desktop – Bildstreifen der Anzeige in den PSRAM
        hw::PIXEL_INTERN.store(false, core::sync::atomic::Ordering::Relaxed);
        pudding_fw::wlan::im_hintergrund_starten();
        // M1: Speicherwacht (Log alle 30 s, Stand für die Absturzmeldung)
        pudding_fw::speicherwacht::starten();
        // JPEG über die Hardware-Einheit (P2); progressiv → Software
        kolibri::dekoder::jpeg_hw_setzen(pudding_fw::jpeg::dekodieren);
        #[cfg(esp32os_testbilder)]
        pudding_fw::testbilder::ablegen_und_messen();
        desktop::shell::anwendungen_setzen(&desktop::anwendungen());
        // Taskmanager (A6): freier Systemspeicher intern / PSRAM
        desktop::taskmanager::speicher_quelle_setzen(|| {
            use esp_idf_svc::sys;
            // SAFETY: reine Abfragen der Heap-Verwaltung.
            unsafe {
                [
                    (sys::heap_caps_get_free_size(sys::MALLOC_CAP_INTERNAL), sys::heap_caps_get_total_size(sys::MALLOC_CAP_INTERNAL)),
                    (sys::heap_caps_get_free_size(sys::MALLOC_CAP_SPIRAM), sys::heap_caps_get_total_size(sys::MALLOC_CAP_SPIRAM)),
                ]
            }
        });
        desktop::shell::haken_setzen(desktop::shell::Haken {
            neustart: || {
                log::info!("Neustart (Startmenü)");
                // SAFETY: Neustart auf Wunsch des Nutzers.
                unsafe { esp_idf_svc::sys::esp_restart() };
            },
            spiel: |id| {
                if let Ok(n) = store::Nvs::open() {
                    n.schreiben(start::START_SCHLUESSEL, &[id]);
                }
                log::info!("Spiel {:?} (Neustart ins Spiel)", SpielId::aus_u8(id));
                // SAFETY: Neustart, das Startziel steht im NVS.
                unsafe { esp_idf_svc::sys::esp_restart() };
            },
        });
    }
    let spielstand = if ziel == SpielId::Wuerfelwelt {
        pudding_fw::daten::einbinden().then(|| std::fs::read(WELT_DATEI).ok()).flatten()
    } else {
        nvs.as_ref().and_then(|n| n.lesen(ziel.nvs_schluessel()))
    };
    log::info!("Startziel {ziel:?}, Spielstand {}", if spielstand.is_some() { "geladen" } else { "neu" });

    // Spielstände schreibt ein eigener Thread: Flash-Schreiben dauert bis zu
    // einige 10 ms, die Bildschleife soll dabei weiterlaufen. Früh starten:
    // Sein Stapel (intern) muss vor den Spielen und der Pipeline Platz haben.
    let (sichern_tx, sichern_rx) = std::sync::mpsc::channel::<(SpielId, Vec<u8>)>();
    let mut sichern_tx = Some(sichern_tx);
    let mut schreiber = Some({
        let nvs = nvs.clone();
        hw::spawn_pinned(c"sichern", 0, 1, 8192, move || {
            for (id, bytes) in sichern_rx {
                let t0 = hw::now_us();
                spielstand_sichern(id, nvs.as_deref(), &bytes);
                log::info!("Spielstand {id:?} gesichert ({} Bytes, {} ms)", bytes.len(), (hw::now_us() - t0) / 1000);
            }
        })
    });

    // --- I²C: Audio-Codec und Touch -----------------------------------------
    let bus = i2c::Bus::new(I2C_PORT, I2C_SDA, I2C_SCL).expect("I2C-Bus");
    if !codec::init(&bus) {
        log::warn!("ES8311 antwortet nicht – kein Ton");
    }
    let touch = match touch::Touch::new(&bus) {
        Ok(t) => {
            log::info!("Touch GT911 unter 0x{:02X}, {}", t.addr, if t.interrupt { "Interrupt" } else { "Abfrage" });
            Some(touch::TouchReader::start(t, None))
        }
        Err(e) => {
            log::error!("Touch GT911 nicht gefunden: {e}");
            None
        }
    };

    // --- Akku (nur, wenn der Spannungsteiler eingetragen ist) ---------------
    let mut battery = BATTERY.and_then(|b| battery::Battery::new(&b).inspect_err(|e| log::error!("Akku: {e}")).ok());

    // --- Ton auf Kern 1 (Synth vorher bauen: ~15 KB, nicht auf dem kleinen
    //     Stapel des Audio-Threads); Doom bringt seinen eigenen Mischer mit.
    let daten = daten_laden(ziel);
    let klang: Box<dyn pudding_core::audio::Klangquelle> = match &daten {
        #[cfg(feature = "doom")]
        Daten::Doom(_) => Box::new(doom::DoomKlang::new()),
        #[cfg(feature = "quake")]
        Daten::Quake(_) => Box::new(quake::QuakeKlang),
        _ => Box::new(audio::MitProgrammen(Synth::new())),
    };
    if let Err(e) = audio::start(klang) {
        log::error!("I2S: {e} – kein Ton");
    }

    // --- Display und Ausgabe --------------------------------------------------
    let mut disp = Display::new(hex(pudding_fw::board::BOOT_COLOR)).expect("Display");
    let mut pipeline = Pipeline::new().expect("PPA");
    // M4: Programm-Pool erst jetzt (Bildspeicher von Anzeige und Pipeline
    // liegen schon) – nur im Desktop (Spiele haben ihre eigenen Abbilder)
    if ziel == SpielId::Konsole {
        pudding_fw::kern_board::pool_anlegen();
    }

    // --- Spiel ----------------------------------------------------------------
    let (aktuell, spiel): (SpielId, Box<dyn Spiel>) = match (ziel, daten) {
        (SpielId::Pudding, _) => {
            let mut app = konsole::pudding::neu(spielstand.as_deref());
            // Ohne Knöpfe: Touch-Steuerung im Spiel immer zeigen.
            app.touch_always = !BUTTONS_ENABLED;
            // Entwicklung: `KONSOLE_AUTOPILOT=1` – Kapitel 1 spielt sich selbst
            // (Messung unter Last ohne Antippen).
            if option_env!("KONSOLE_AUTOPILOT").is_some() {
                app.start_chapter(0, pudding_core::campaign::Difficulty::Easy);
                app.set_autopilot(true);
                if let Some(w) = option_env!("KONSOLE_WEICH") {
                    app.save.settings.smooth = w == "1";
                }
            }
            (ziel, Box::new(app))
        }
        (SpielId::Wuerfelwelt, _) => {
            // Viele kleine Netze: ab 64 Byte in den PSRAM, damit der interne
            // RAM den Bildstreifen bleibt (gilt nur in diesem Start).
            // SAFETY: reine Einstellung des Heaps.
            unsafe { esp_idf_svc::sys::heap_caps_malloc_extmem_enable(64) };
            let t0 = hw::now_us();
            let mut w = wuerfelwelt::Wuerfelwelt::neu(spielstand.as_deref());
            w.autopilot = option_env!("KONSOLE_AUTOPILOT").is_some();
            log::info!("Würfelwelt erzeugt in {} ms", (hw::now_us() - t0) / 1000);
            (ziel, Box::new(w))
        }
        #[cfg(feature = "quake")]
        (SpielId::Quake, Daten::Quake(pak)) => {
            quake_board::ordner_anlegen();
            let t0 = hw::now_us();
            // „Quit“ im Quake-Menü: zurück in die Konsole (Neustart).
            quake::beim_beenden(|| unsafe { esp_idf_svc::sys::esp_restart() });
            let q = quake::Quake::starten_mit(pak, quake_board::PAK_DATEI, quake_board::ORDNER, quake_starter);
            log::info!("Quake gestartet in {} ms", (hw::now_us() - t0) / 1000);
            log_heap("Quake geladen");
            (ziel, Box::new(q))
        }
        #[cfg(feature = "doom")]
        (SpielId::Doom, Daten::Doom(wad)) => {
            doom_board::ordner_anlegen();
            let t0 = hw::now_us();
            let d = doom::Doom::starten(wad, doom_board::ORDNER);
            log::info!("Doom gestartet in {} ms", (hw::now_us() - t0) / 1000);
            log_heap("Doom geladen");
            (ziel, Box::new(d))
        }
        _ if desktop_start => {
            kern::fenster::bildschirm(desktop::BREITE as i32, desktop::HOEHE as i32);
            if let Err(e) = kern::skin::anwenden(kern::skin::STANDARD, 2) {
                log::error!("Skin: {e}");
            }
            // Absturz vor dem Neustart? Dann melden und kein Testprogramm starten.
            let absturz = pudding_fw::absturz::holen();
            if let Err(e) = kern::starte_name("@start") {
                log::error!("@start: {e:?}");
            }
            let prog = option_env!("ESP32OS_START").unwrap_or("@start");
            // Doom/Quake konnten ihre Spieldaten nicht laden → Hinweis (S5)
            if let Some(n) = nvs.as_deref()
                && let Some(id) = n.lesen(desktop::shell::SPIEL_FEHLER_SCHLUESSEL)
            {
                n.loeschen(desktop::shell::SPIEL_FEHLER_SCHLUESSEL);
                if let Some(id) = id.first().copied().and_then(SpielId::aus_u8) {
                    log::warn!("{id:?}: Spieldaten unbrauchbar – Hinweis im Desktop");
                    let (titel, text) = desktop::shell::spieldaten_hinweis(id, true);
                    desktop::shell::melden(titel, &text);
                }
            }
            if let Some((name, grund)) = &absturz {
                log::warn!("Letzter Neustart nach Absturz von „{name}“: {grund}");
                desktop::shell::absturz_melden(name, grund);
            } else if prog != "@start" {
                // „programm parameter“ (z. B. „bildbetrachter /rd/1/bilder/foto.jpg“),
                // mehrere mit „;“ getrennt (z. B. „endlostest;taskmanager“)
                for prog in prog.split(';').map(str::trim).filter(|p| !p.is_empty()) {
                    let (name, param) = prog.split_once(' ').unwrap_or((prog, ""));
                    let r = kern::datei::starten(&format!("/sys/{name}"), param);
                    if r < 0 {
                        log::error!("Programm {prog}: Fehler {}", -r);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            }
            // S4 (Entwicklung, einmal am Board des Nutzers): Ersteinrichtung als
            // erledigt merken (Merker setup=1), ohne sie zu bedienen
            if option_env!("ESP32OS_SETUP_ERLEDIGT").is_some() {
                log::warn!("Ersteinrichtung als erledigt gemerkt: {}", desktop::konfig::einrichtung_abschliessen());
            }
            log::info!("ESP32-OS-Desktop startet ({prog})");
            let mut d = desktop::Desktop::neu(|| hw::now_us() as u64);
            if let Some(a) = AUTOPILOT {
                d.autopilot(a);
            }
            (SpielId::Konsole, Box::new(d) as Box<dyn Spiel>)
        }
        _ => {
            // Den alten Launcher gibt es nicht mehr (S0). Hierher kommen nur
            // Selbsttest-Builds (bleiben stehen) und Doom/Quake, deren
            // Spieldaten sich nicht laden liessen (beschädigt, abgeschnitten):
            // zurück in den Desktop, der den Hinweis zeigt (S-Review).
            if selbsttest || dateitest {
                log::info!("Selbsttest fertig – das Board bleibt stehen");
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
            log::error!("{ziel:?}: Spieldaten nicht ladbar – zurück in den Desktop");
            if ziel != SpielId::Konsole
                && let Some(n) = nvs.as_deref()
            {
                n.schreiben(desktop::shell::SPIEL_FEHLER_SCHLUESSEL, &[ziel.als_u8()]);
            }
            slots::umschalten(SpielId::Konsole);
            // SAFETY: Neustart in den Desktop; der Merker steht im NVS.
            unsafe { esp_idf_svc::sys::esp_restart() };
            #[allow(unreachable_code)]
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    };
    let mut rahmen = Rahmen::new(spiel);
    // Entwicklung: `KONSOLE_RUNDGANG=1` beim Bauen – jedes Spiel nach 6 s auf
    // dem normalen Weg verlassen (Konsole → Pudding → Doom → Würfelwelt →
    // Konsole), um den Spielwechsel ohne Antippen zu prüfen.
    let rundgang = option_env!("KONSOLE_RUNDGANG").and_then(|_| {
        let n = nvs.as_ref()?;
        let schritt = n.lesen("rundgang").and_then(|b| b.first().copied()).unwrap_or(0);
        if schritt >= 5 {
            n.loeschen("rundgang");
            log::info!("Rundgang beendet");
            return None;
        }
        n.schreiben("rundgang", &[schritt + 1]);
        Some(match aktuell {
            SpielId::Konsole => SpielId::Pudding,
            SpielId::Pudding => SpielId::Doom,
            SpielId::Doom => SpielId::Wuerfelwelt,
            SpielId::Wuerfelwelt => SpielId::Quake,
            SpielId::Quake => SpielId::Konsole,
        })
    });
    let rundgang_start = hw::now_us();
    disp.set_brightness(rahmen.helligkeit());
    let mut pacer = FramePacer::new(PANEL_HZ);
    let mut dt = pacer.period();
    let mut fps = 0.0f32;
    let mut frames = 0u32;
    let mut last_log = hw::now_us();
    let mut battery_t = 0u32;
    let (mut sum_sim, mut sum_prep, mut sum_wait, mut sum_fin, mut sum_n) = (0i64, 0i64, 0i64, 0i64, 0u32);
    let (mut sum_tris, mut sum_join) = (0i64, 0i64);
    let mut perf = Perf::default();
    // Entwicklung: `KONSOLE_BILDER=10,25` – Bildschirmfotos (Sekunden nach
    // dem Start) über die serielle Schnittstelle, siehe `foto.rs`.
    let foto_zeiten: Vec<i64> = option_env!("KONSOLE_BILDER")
        .map(|s| s.split(',').filter_map(|t| t.trim().parse::<f32>().ok()).map(|t| (t * 1e6) as i64).collect())
        .unwrap_or_default();
    let (mut foto_nr, foto_start) = (0usize, hw::now_us());
    // Vorher/Nachher-Messungen: Bilanz alle 30 s (siehe `Bildbilanz`).
    let mut bilanz = pudding_core::present::Bildbilanz::default();
    let mut bilanz_start = hw::now_us();
    // Ruckler-Messung je 2-s-Fenster: verpasste Bildwechsel, längstes Bild
    // und seine Anteile (Simulation, Vorbereiten, Mithelfen, Sichern).
    let (mut verpasst, mut laengstes) = (0u32, (0i64, 0i64, 0i64, 0i64, 0i64));
    // Bildschirmschoner und Ruhezustand (siehe Schleife; Zeiten vom Spiel,
    // im Desktop aus den Einstellungen).
    let mut letzte_eingabe = hw::now_us();
    let (mut schoner_dunkel, mut schoner_weckt) = (false, false);
    let mut ruhe = false;
    // Klemmender Touch: (Punkt, seit, gemeldet/ignoriert)
    let mut klemmt: Option<((u16, u16), i64, bool)> = None;
    // Ein Bild ist beauftragt, aber noch nicht angezeigt.
    let mut pending = false;
    // Das Bild, das die Kerne gerade zeichnen, und das nächste, das das
    // Spiel währenddessen vorbereitet.
    let mut drawing = FrameData::new();
    let mut naechstes = FrameData::new();
    let mut stats = FrameStats::default();
    // Geänderte Zeilen der letzten Bilder (O1): Die Anzeige hat FB_COUNT
    // Bildspeicher reihum – ein Speicher hält das Bild von vor FB_COUNT
    // Bildern. Ausgegeben werden nur Streifen, die sich seither geändert
    // haben (Vereinigung über die letzten FB_COUNT Bilder); None = alles.
    let mut zeilen_verlauf: [Option<(usize, usize)>; pudding_fw::display::FB_COUNT] = [None; pudding_fw::display::FB_COUNT];
    let mut desktop_log = hw::now_us();
    log_heap("Spiel geladen");
    if slots::hier(SpielId::Konsole) {
        // ESP32-OS ist bis zur Anzeigeschleife gekommen: gültig, Platz merken
        slots::als_gueltig_markieren();
        slots::os_platz_merken();
        if option_env!("ESP32OS_OTA_TEST").is_some() {
            slots::ota_test();
        }
    }
    log::info!("Los geht's!");

    // Fehlersuche: meldet jede Sekunde, wo die Schleife steht, falls sie
    // nicht mehr weiterkommt (höhere Priorität als die Spielschleife).
    loop {
        let start = hw::now_us();
        LOOP_FRAMES.fetch_add(1, Ordering::Relaxed);
        LOOP_STAGE.store(1, Ordering::Relaxed);

        // 1) Eingaben (Touch in das Raster des Spiels umrechnen).
        let flip = rahmen.gedreht();
        let raster = rahmen.touch_raster();
        let mut input = Input::buttons(pudding_fw::read_buttons());
        if let Some(t) = &touch {
            let roh = t.take();
            // Ein Punkt, der 60 s unverändert liegt, wird nur gemeldet (Log).
            // Die frühere Ursache (verlorene „losgelassen“-Meldung im Treiber)
            // ist in touch.rs behoben (V1); ausgeblendet wird nichts mehr.
            let erster = roh.first().copied().flatten();
            match (erster, klemmt) {
                (Some(p), Some((q, _, _))) if p.0.abs_diff(q.0) <= 3 && p.1.abs_diff(q.1) <= 3 => {}
                (Some(p), _) => klemmt = Some((p, start, false)),
                (None, _) => {
                    if klemmt.is_some_and(|k| k.2) {
                        log::info!("Touch: lange liegender Punkt losgelassen");
                    }
                    klemmt = None;
                }
            }
            if let Some((p, seit, gemeldet)) = klemmt.as_mut()
                && start - *seit > 60_000_000
            {
                if !*gemeldet {
                    log::warn!("Touch: Punkt bei Panel ({}, {}) liegt seit 60 s unverändert", p.0, p.1);
                    *gemeldet = true;
                }
            }
            // Autopilot-Builds: nur die eingespielten Berührungen zählen
            if AUTOPILOT.is_none() {
                let mut i = 0;
                for p in roh.into_iter().flatten() {
                    if let Some(slot) = input.touch.get_mut(i) {
                        *slot = Some(panel_to_raster(p, LCD_W as u16, LCD_H as u16, flip, raster));
                        i += 1;
                    }
                }
            }
        }
        // Entwicklung (S1–S4): `ESP32OS_SPIEL_TIPPEN="35:30:30,40:160:123"` –
        // in einem Spiel s Sekunden nach dem Start bei (x, y) im Raster des
        // Spiels 0,3 s antippen (wie ein Finger; (30, 30) = Home-Taste des
        // Rahmens) – Nachweis „zurück in den Desktop“
        if aktuell != SpielId::Konsole
            && let Some(liste) = option_env!("ESP32OS_SPIEL_TIPPEN")
        {
            for e in liste.split(',') {
                let v: Vec<i64> = e.split(':').filter_map(|z| z.trim().parse().ok()).collect();
                if let [s, x, y] = v[..]
                    && (s * 1_000_000..s * 1_000_000 + 300_000).contains(&start)
                {
                    input.touch[0] = Some((x as u16, y as u16));
                }
            }
        }
        // Bildschirmschoner (P5): nach der eingestellten Zeit ohne Eingabe
        // Hintergrundlicht aus, danach Ruhezustand (Panel schläft, Schleife
        // gedrosselt); die Berührung zum Aufwecken erreicht das Spiel nicht.
        // Autopilot (Spiel-eigene Eingabe) zählt wie eine Berührung
        let fremd = rahmen.fremde_eingabe();
        let beruehrt = input.touching() || input.held != 0 || fremd;
        // Programm will den Bildschirm an (Wecker klingelt): aufwecken wie
        // eine Berührung, aber ohne die Eingabe zu sperren (Stopp antippen)
        if !beruehrt && rahmen.wach_halten() {
            if schoner_dunkel {
                if ruhe {
                    let (a, b) = disp.schlafen(false);
                    log::info!("Ruhezustand: vom Programm geweckt (Panel {a}/{b})");
                    ruhe = false;
                } else {
                    log::info!("Bildschirmschoner: vom Programm geweckt");
                }
                schoner_dunkel = false;
                pudding_fw::wlan::SPAREN.store(false, Ordering::Relaxed);
            }
            letzte_eingabe = start;
        }
        if beruehrt {
            if schoner_dunkel {
                schoner_weckt = true;
                if ruhe {
                    let (a, b) = disp.schlafen(false);
                    // C3: WLAN-Zustand beim Aufwachen (Nachweis „WLAN wieder da“)
                    let z = kern::wlan::zustand();
                    log::info!(
                        "Ruhezustand: aufgeweckt (Panel {a}/{b}) nach {} s, WLAN {}",
                        (start - letzte_eingabe) / 1_000_000,
                        if z.verbunden { "verbunden" } else if z.verbindet { "verbindet" } else { "getrennt" }
                    );
                    ruhe = false;
                } else {
                    log::info!("Bildschirmschoner: aufgeweckt");
                }
            }
            schoner_dunkel = false;
            pudding_fw::wlan::SPAREN.store(false, Ordering::Relaxed);
            letzte_eingabe = start;
        } else {
            schoner_weckt = false;
        }
        if schoner_weckt {
            input = Input::default();
        }
        rahmen.eingabe_sperren(schoner_weckt);
        // Entwicklung: `ESP32OS_SCHONER_S=25` – Schoner nach 25 s (nur dieser
        // Build, die Einstellung bleibt unverändert; Test von W5)
        let aus_us = option_env!("ESP32OS_SCHONER_S")
            .and_then(|s| s.parse::<i64>().ok())
            .or_else(|| rahmen.bildschirm_aus_s().map(i64::from))
            .map(|s| s * 1_000_000);
        if !schoner_dunkel && aus_us.is_some_and(|a| start - letzte_eingabe > a) {
            schoner_dunkel = true;
            pudding_fw::wlan::SPAREN.store(true, Ordering::Relaxed);
            log::info!("Bildschirmschoner: Hintergrundlicht aus");
        }
        let ruhe_us = aus_us.zip(rahmen.ruhe_nach_s()).map(|(a, r)| a + i64::from(r) * 1_000_000);
        let jetzt_ruhe = rahmen.ruhe_jetzt();
        if !ruhe && (jetzt_ruhe || (schoner_dunkel && ruhe_us.is_some_and(|r| start - letzte_eingabe > r))) {
            if pending {
                stats = pipeline.join();
                pipeline.finish();
                let _ = disp.present_nach(rahmen.bildrate_teiler());
                pending = false;
            }
            disp.set_brightness(0);
            pudding_fw::wlan::SPAREN.store(true, Ordering::Relaxed);
            let (a, b) = disp.schlafen(true);
            log::info!("Ruhezustand: Panel schläft ({a}/{b}){}", if jetzt_ruhe { ", aus dem Menü" } else { "" });
            schoner_dunkel = true;
            ruhe = true;
        }
        // Ruhezustand: nichts zeichnen, nur alle 50 ms nach Berührung sehen
        // (Programme laufen weiter, z. B. Uhr und Wecker)
        if ruhe {
            std::thread::sleep(std::time::Duration::from_millis(50));
            dt = pacer.period();
            continue;
        }

        // 2) Simulation (fester Takt im Kern), Ton, Spielstand, Helligkeit.
        let t_sim = hw::now_us();
        LOOP_STAGE.store(2, Ordering::Relaxed);
        let wechsel = rahmen.update(&input, dt);
        let wechsel = match rundgang {
            Some(z) if start - rundgang_start > 6_000_000 => Wechsel::Starte(z),
            _ => wechsel,
        };
        LOOP_STAGE.store(3, Ordering::Relaxed);
        let sim_us = hw::now_us() - t_sim;
        rahmen.ton(&mut |c| {
            audio::QUEUE.push(c);
        });
        let t_sichern = hw::now_us();
        if let Some(bytes) = rahmen.speichern()
            && !MESSLAUF
            && let Some(tx) = &sichern_tx
        {
            let _ = tx.send((aktuell, bytes));
        }
        let sichern_us = hw::now_us() - t_sichern;
        if let Wechsel::Starte(nach) = wechsel {
            log::info!("Wechsel {aktuell:?} -> {nach:?} (Neustart)");
            rahmen.beenden();
            if let Some(bytes) = rahmen.speichern()
                && !MESSLAUF
                && let Some(tx) = &sichern_tx
            {
                let _ = tx.send((aktuell, bytes));
            }
            // Warten, bis alles geschrieben ist.
            drop(sichern_tx.take());
            if let Some(t) = schreiber.take() {
                let _ = t.join();
            }
            if let Some(n) = nvs.as_deref() {
                start::wechsel_vorbereiten(n, aktuell, nach);
            }
            if !slots::hier(nach) {
                slots::umschalten(nach);
            }
            if pending {
                pipeline.join();
                pipeline.finish();
            }
            // SAFETY: Neustart; alles Nötige ist gespeichert.
            unsafe { esp_idf_svc::sys::esp_restart() };
        }
        disp.set_brightness(if schoner_dunkel { 0 } else { rahmen.helligkeit() });
        if let Some(b) = battery.as_mut() {
            battery_t += 1;
            if battery_t % 110 == 1 {
                // ~alle 2 s
                rahmen.akku(b.read_mv().and_then(pudding_core::battery::percent));
            }
        }

        // 3) Nächstes Bild vorbereiten (Zeichenliste, Sortieren, Dreiecke,
        //    HUD) – Kern 1 zeichnet währenddessen schon das vorige.
        let (smooth, flip) = (rahmen.glatt(), rahmen.gedreht());
        let (w, h) = rahmen.render_size();
        let t_prep = hw::now_us();
        LOOP_STAGE.store(4, Ordering::Relaxed);
        rahmen.bild(&mut naechstes, &perf);
        LOOP_STAGE.store(5, Ordering::Relaxed);
        let prep_us = hw::now_us() - t_prep;
        if desktop_start && start - desktop_log >= 2_000_000 {
            let m = desktop::messung_holen();
            let s = (start - desktop_log) as f32 * 1e-6;
            log::info!(
                "Desktop: {:.1} Bilder/s neu, {:.1} übersprungen/s, Fläche {:.0} px/Bild, Compositor {:.2} ms/Bild",
                m.bilder as f32 / s,
                m.leer as f32 / s,
                m.flaeche as f32 / m.bilder.max(1) as f32,
                m.zeit_us as f32 / 1000.0 / (m.bilder + m.leer).max(1) as f32
            );
            desktop_log = start;
        }
        // Desktop ohne Änderung: nichts zeichnen, das letzte Bild bleibt stehen.
        if rahmen.unveraendert() {
            if foto_nr < foto_zeiten.len() && hw::now_us() - foto_start >= foto_zeiten[foto_nr] && !pending {
                pudding_fw::foto::senden(disp.gezeigt(), foto_nr as u32, rahmen.gedreht());
                foto_nr += 1;
            }
            if pending {
                stats = pipeline.join();
                pipeline.finish();
                let _ = disp.present_nach(rahmen.bildrate_teiler());
                pending = false;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
            dt = pacer.period();
            continue;
        }
        let prims = naechstes.prims();

        // 4) Beim vorigen Bild mithelfen, bis alle Streifen vergeben sind; dann
        //    ab dem nächsten Bildanfang anzeigen. Spielzeit für das nächste
        //    Bild aus den Bildwechseln.
        let (mut finish_us, mut join_us) = (0u32, 0i64);
        let mut v_bild = 0u32;
        if pending {
            let t_join = hw::now_us();
            LOOP_STAGE.store(6, Ordering::Relaxed);
            stats = pipeline.join();
            join_us = hw::now_us() - t_join;
            LOOP_STAGE.store(7, Ordering::Relaxed);
            finish_us = pipeline.finish();
            LOOP_STAGE.store(8, Ordering::Relaxed);
            let shown_at = disp.present_nach(rahmen.bildrate_teiler());
            dt = pacer.frame(shown_at, hw::now_us());
            v_bild = ((dt / pacer.period()).round() as u32).saturating_sub(rahmen.bildrate_teiler());
            verpasst += v_bild;
            if foto_nr < foto_zeiten.len() && hw::now_us() - foto_start >= foto_zeiten[foto_nr] {
                pudding_fw::foto::senden(disp.gezeigt(), foto_nr as u32, rahmen.gedreht());
                foto_nr += 1;
            }
        }

        // 5) Das vorbereitete Bild übernehmen und zeichnen lassen: Kern 1
        //    beginnt sofort, Kern 0 simuliert erst das nächste Bild.
        core::mem::swap(&mut drawing, &mut naechstes);
        let wait_start = hw::now_us();
        LOOP_STAGE.store(9, Ordering::Relaxed);
        let fb = disp.acquire();
        LOOP_STAGE.store(10, Ordering::Relaxed);
        let wait_us = hw::now_us() - wait_start;
        zeilen_verlauf.rotate_right(1);
        zeilen_verlauf[0] = rahmen.geaenderte_zeilen();
        let zeilen = zeilen_verlauf.iter().try_fold(None::<(usize, usize)>, |acc, z| {
            z.map(|(a, b)| Some(acc.map_or((a, b), |(c, d)| (c.min(a), d.max(b)))))
        }).flatten();
        // SAFETY: `drawing` und `fb` bleiben bis zum `join` oben unverändert.
        unsafe { pipeline.start(&drawing, Frame { w, h, smooth, flip, fb, zeilen }) };
        pending = true;

        let now = hw::now_us();
        let inst = 1.0 / dt;
        fps = if fps == 0.0 { inst } else { fps + (inst - fps) * 0.1 };
        perf = Perf {
            fps,
            render_ms: stats.render_ms(),
            scale_ms: stats.scale_ms(),
            // Arbeitszeit ohne Warten auf den freien Bildspeicher (VSync).
            frame_ms: (now - start - wait_us) as f32 / 1000.0,
        };

        // 6) Alle 2 s eine Zeile in den seriellen Monitor.
        frames = frames.wrapping_add(1);
        sum_fin += finish_us as i64;
        sum_tris += prims as i64;
        sum_join += join_us;
        sum_sim += sim_us;
        sum_prep += prep_us;
        sum_wait += wait_us;
        sum_n += 1;
        let dauer = now - start - wait_us;
        bilanz.bild(dauer as u32, v_bild);
        if now - bilanz_start >= 30_000_000 {
            let s = (now - bilanz_start) as f32 * 1e-6;
            log::info!(
                "Bilanz {:.0} s: {:.1} FPS, Arbeit je Bild Mittel {:.1} ms, p99 {:.1} ms, max {:.1} ms; verpasste Bildwechsel {} ({:.1} je 100)",
                s,
                bilanz.bilder() as f32 / s,
                bilanz.mittel_ms(),
                bilanz.perzentil_ms(0.99),
                bilanz.max_ms(),
                bilanz.verpasst(),
                bilanz.verpasst() as f32 * 100.0 / (s / pacer.period()).max(1.0)
            );
            bilanz = pudding_core::present::Bildbilanz::default();
            bilanz_start = now;
        }
        if dauer > laengstes.0 {
            laengstes = (dauer, sim_us, prep_us, join_us, sichern_us);
        }
        if now - last_log >= 2_000_000 {
            last_log = now;
            let n = sum_n.max(1) as f32 * 1000.0;
            log::info!(
                "Mittel: Simulation {:.1} ms, Bild vorbereiten {:.1} ms ({} Dreiecke), Mithelfen {:.1} ms, Rest-PPA {:.1} ms, Warten auf Bildspeicher {:.1} ms",
                sum_sim as f32 / n,
                sum_prep as f32 / n,
                sum_tris / sum_n.max(1) as i64,
                sum_join as f32 / n,
                sum_fin as f32 / n,
                sum_wait as f32 / n
            );
            (sum_sim, sum_prep, sum_wait, sum_fin, sum_tris, sum_join, sum_n) = (0, 0, 0, 0, 0, 0, 0);
            log::info!(
                "Ruckler: {} verpasste Bildwechsel; längstes Bild {:.1} ms (Simulation {:.1}, Vorbereiten {:.1}, Mithelfen {:.1}, Sichern {:.1})",
                verpasst,
                laengstes.0 as f32 / 1000.0,
                laengstes.1 as f32 / 1000.0,
                laengstes.2 as f32 / 1000.0,
                laengstes.3 as f32 / 1000.0,
                laengstes.4 as f32 / 1000.0
            );
            (verpasst, laengstes) = (0, (0, 0, 0, 0, 0));
            let (int_kb, psram_kb) = hw::free_kb();
            // Programme brauchen ihren Heap am Stück: grösster freier Block
            // SAFETY: reine Abfrage des Heaps.
            let psram_block_kb = unsafe { esp_idf_svc::sys::heap_caps_get_largest_free_block(esp_idf_svc::sys::MALLOC_CAP_SPIRAM) } / 1024;
            let bp = &pudding_core::render::BUILD_PROFILE;
            let take = |i: usize| bp[i].swap(0, core::sync::atomic::Ordering::Relaxed) as f32;
            let (tv, tt, nt) = (take(0), take(1), take(2).max(1.0));
            let (tc, nc, no) = (take(6), take(7).max(1.0), take(8));
            log::info!(
                "Geclippt {:.0} je Bild ({:.1} ms); {:.0} Objekte, {:.0} Dreiecke geprüft; Ton {:.1} ms je Bild",
                nc / n * 1000.0,
                tc / 360_000.0 / n * 1000.0,
                no / n * 1000.0,
                nt / n * 1000.0,
                pudding_fw::audio::RENDER_US.swap(0, Ordering::Relaxed) as f32 / n
            );
            log::info!(
                "Frei: {int_kb} KB intern, {psram_kb} KB PSRAM (grösster Block {psram_block_kb} KB); Aufbereiten je Bild: Vertices {:.1} ms, Dreiecke {:.1} ms ({:.0} Takte je Dreieck)",
                tv / 360_000.0 / n * 1000.0,
                tt / 360_000.0 / n * 1000.0,
                tt / nt
            );
            let rp = &pudding_core::render::RENDER_PROFILE;
            let rt = |i: usize| rp[i].swap(0, core::sync::atomic::Ordering::Relaxed) as f32;
            let (rc, rr, ro, rows, px) = (rt(0), rt(1), rt(2), rt(3), rt(4));
            let per = |x: f32| x / 360_000.0 / n * 1000.0 / 2.0;
            log::info!(
                "Rendern je Kern: Löschen {:.1} ms, Dreiecke {:.1} ms, HUD {:.1} ms; je Bild {:.0} Zeilenstücke, {:.0}k Pixel ({:.1} Takte/Pixel)",
                per(rc),
                per(rr),
                per(ro),
                rows / n * 1000.0,
                px / n,
                rr / px.max(1.0)
            );
            let sub = |i: usize| {
                pudding_fw::pipeline::SUBMIT_US[i].swap(0, core::sync::atomic::Ordering::Relaxed) as f32 / n
            };
            log::info!(
                "PPA-Aufrufe je Bild: Kern 0 {:.2} ms, Kern 1 {:.2} ms; angeschubst seit Start: {}",
                sub(0),
                sub(1),
                pudding_fw::pipeline::KICKS.load(Ordering::Relaxed)
            );
            let [c0, c1] = stats.cores;
            log::info!(
                "{:.1} FPS ({}x{} {}), Frame {:.1} ms, Streifen {}/{}, Rendern {:.1}/{:.1} ms, Skalieren {:.1}/{:.1} ms, \
                 PPA {:.1}/{:.1} ms, Ausgabe {:.1} ms, Panel {:.2} Hz",
                fps,
                w,
                h,
                if smooth { "weich" } else { "scharf" },
                perf.frame_ms,
                c0.strips,
                c1.strips,
                c0.render_us as f32 / 1000.0,
                c1.render_us as f32 / 1000.0,
                c0.scale_us as f32 / 1000.0,
                c1.scale_us as f32 / 1000.0,
                c0.output_us as f32 / 1000.0,
                c1.output_us as f32 / 1000.0,
                stats.wall_us as f32 / 1000.0,
                1.0 / pacer.period()
            );
            if frames < 400 {
                let [p0, p1] = pipeline.placement;
                log::info!(
                    "Bildpuffer: Kern 0 {} KB intern/{} KB PSRAM, Kern 1 {} KB intern/{} KB PSRAM",
                    p0.internal_kb,
                    p0.psram_kb,
                    p1.internal_kb,
                    p1.psram_kb
                );
            }
        }
    }
}

/// Spieldaten aus dem Flash, die schon der Ton braucht (Doom: WAD, Quake: PAK).
enum Daten {
    Keine,
    #[cfg(feature = "doom")]
    Doom(&'static [u8]),
    #[cfg(feature = "quake")]
    Quake(quake::Pak),
}

fn daten_laden(ziel: SpielId) -> Daten {
    match ziel {
        #[cfg(feature = "doom")]
        SpielId::Doom => doom_board::wad_laden().map(Daten::Doom).unwrap_or(Daten::Keine),
        #[cfg(feature = "quake")]
        SpielId::Quake => quake_board::pak().map(Daten::Quake).unwrap_or(Daten::Keine),
        _ => Daten::Keine,
    }
}

/// Quake läuft in einem eigenen Thread mit großem Stapel im PSRAM (Kern 0).
#[cfg(feature = "quake")]
fn quake_starter(f: Box<dyn FnOnce() + Send + 'static>) {
    hw::spawn_pinned_psram(c"quake", 0, 1, quake::STAPEL, f);
}

/// Spielstand der Würfelwelt (größer als ein NVS-Eintrag): Datei auf „daten“.
const WELT_DATEI: &str = "/daten/welt.bin";

/// Spielstand sichern: Würfelwelt als Datei, sonst im NVS (Schlüssel je Spiel).
fn spielstand_sichern(id: SpielId, nvs: Option<&store::Nvs>, bytes: &[u8]) {
    if id == SpielId::Wuerfelwelt {
        if pudding_fw::daten::einbinden()
            && let Err(e) = std::fs::write(WELT_DATEI, bytes)
        {
            log::error!("Würfelwelt speichern: {e}");
        }
    } else if let Some(n) = nvs {
        n.schreiben(id.nvs_schluessel(), bytes);
    }
}

/// Freier interner RAM (Fehlersuche: Thread-Stacks und Treiber brauchen ihn).
fn log_heap(when: &str) {
    use esp_idf_svc::sys;
    // SAFETY: reine Abfragen.
    unsafe {
        log::info!(
            "{when}: intern frei {} KB (größter Block {} KB), DMA-fähig {} KB",
            sys::heap_caps_get_free_size(sys::MALLOC_CAP_INTERNAL) / 1024,
            sys::heap_caps_get_largest_free_block(sys::MALLOC_CAP_INTERNAL) / 1024,
            sys::heap_caps_get_free_size(sys::MALLOC_CAP_DMA) / 1024,
        );
    }
}
