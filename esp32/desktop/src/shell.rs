//! ESP32-OS-Shell: Startbild (`@start`), Desktop-Symbole (`@icon`),
//! Taskleiste (`@taskbar`), Startmenü (`@menu`) und Info-Dialog (`info`) –
//! eigene Programme wie in KolibriOS, über die Systemfunktionen des Kerns.
//! Original-Symbole aus KolibriOS (`daten/icons32.png`), für Touch
//! vergrössert (Symbole 48 px, Zeilen ≥ 40 px bei logisch 640×360 ≙ ×2).

use kern::sys;
use kolibri::dekoder::Rgba;
use std::sync::Mutex;

/// NVS-Schlüssel des früheren Merkers „Spiele“ (Neustart in den alten
/// Launcher); den Launcher gibt es nicht mehr – die Firmware löscht ihn nur.
pub const MODUS_SCHLUESSEL: &str = "esp32os_modus";

/// Höhe der Taskleiste (logische Pixel; Knöpfe 41 px ≈ 7,1 mm).
pub const LEISTE_H: i32 = 48;
/// Name des Systems (Startbild, Info, Taskleiste).
pub const NAME: &str = "ESP32-OS";
pub const VERSION: &str = "1.0.4";

/// Eintrag im Startmenü und auf dem Desktop.
#[derive(Clone, Copy, Debug)]
pub struct Anwendung {
    /// Programmname (kern::ProgrammEintrag).
    pub programm: &'static str,
    /// Titel englisch / deutsch (Sprache: [`Anwendung::name`]).
    pub titel: &'static str,
    pub titel_de: &'static str,
    /// Index in icons32.png.
    pub symbol: usize,
    pub auf_desktop: bool,
}

impl Anwendung {
    /// Titel in der aktuellen Sprache.
    pub fn name(&self) -> &'static str {
        kern::sprache::t(self.titel, self.titel_de)
    }
}

/// Haken der Plattform (Board: Neustart, Spielekonsole). Am PC leer.
#[derive(Clone, Copy)]
pub struct Haken {
    pub neustart: fn(),
    /// Spiel starten (Neustart mit Startziel, `konsole::SpielId` als u8).
    pub spiel: fn(u8),
}

static HAKEN: Mutex<Option<Haken>> = Mutex::new(None);
static ANWENDUNGEN: Mutex<Vec<Anwendung>> = Mutex::new(Vec::new());

pub fn haken_setzen(h: Haken) {
    *HAKEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(h);
}

/// Anwendungen für Menü und Desktop festlegen (Systemstart).
pub fn anwendungen_setzen(v: &[Anwendung]) {
    kern::kern_abschnitt(|| *ANWENDUNGEN.lock().unwrap_or_else(|e| e.into_inner()) = v.to_vec());
}

fn anwendungen() -> Vec<Anwendung> {
    // Sperre nur im Kern-Abschnitt (siehe SYMBOL_CACHE; Zwangsbeenden)
    // Kopie im Heap des Aufrufers (geschützt, nicht im Kern-Abschnitt): eine
    // Kopie im System-Heap bliebe liegen, wenn das Programm ohne Aufräumen
    // endet (M8, Dauertest 26.11.)
    sperre_vorbereiten(&ANWENDUNGEN);
    kern::geschuetzt(|| ANWENDUNGEN.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

fn haken() -> Option<Haken> {
    kern::kern_abschnitt(|| *HAKEN.lock().unwrap_or_else(|e| e.into_inner()))
}

// ------------------------------------------------------------ Symbole

/// Symbole einzeln aus `daten/icons32.png`, beim Bauen zerlegt (build.rs,
/// O1): entpackt wird nur, was gezeigt wird.
mod erzeugt {
    include!(concat!(env!("OUT_DIR"), "/symbole.rs"));
}

/// Symbol `i` aus der Tabelle `t` (Kantenlänge `g`) entpacken.
fn symbol_entpacken(t: &[(usize, usize)], i: usize, g: u32) -> Rgba {
    let n = (g * g) as usize;
    let (a, l) = t[i];
    let px = kolibri::dekoder::inflate(&erzeugt::SYMBOLE_BLOB[a..a + l], 4 * n)
        .ok()
        .filter(|d| d.len() == 4 * n)
        .map(|d| d.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
        .unwrap_or_else(|| vec![0; n]);
    Rgba { w: g, h: g, px, delay: 0 }
}

/// Bereits skalierte Symbole (System-Heap; Skalieren kostet am Board viel
/// Zeit und verzögert das Neuzeichnen – währenddessen gehen Tipps verloren).
/// Nur im Kern-Abschnitt sperren: am Board legt `std::Mutex` sein
/// pthread-Objekt beim ersten Sperren im Heap des Aufrufers an – im
/// Programm-Heap wäre es nach dem Programmende weg (Absturz 22.11.).
static SYMBOL_CACHE: Mutex<Vec<(usize, u32, Rgba)>> = Mutex::new(Vec::new());

/// Symbol `i` in Grösse `g`.
/// Die Mutex der Standardbibliothek legt ihr Sperrobjekt beim ersten Sperren
/// an – im Heap dessen, der sperrt. Gemeinsame Mutexe deshalb zuerst im
/// Kern-Abschnitt sperren (Objekt im System-Heap); sonst läge es im Heap
/// eines Programms und zeigte nach dessen Ende ins Leere („failed to lock
/// mutex: Invalid argument“, Dauertest 26.11.).
pub(crate) fn sperre_vorbereiten<T>(m: &std::sync::Mutex<T>) {
    kern::kern_abschnitt(|| drop(m.lock()));
}

pub fn symbol(i: usize, g: u32) -> Rgba {
    let i = i.min(erzeugt::SYMBOLE_ANZAHL - 1);
    // M8 (Dauertest 26.11.): die Kopie für das Programm entsteht in seinem
    // eigenen Heap (geschützter Abschnitt). Vorher im Kern-Abschnitt, also im
    // System-Heap – endete das Programm ohne Aufräumen (Startmenü nach dem
    // Start eines Programms), blieb sie liegen: 12 Symbole × 9216 Bytes je
    // Menü, ~1 MB je Stunde im Dauertest.
    sperre_vorbereiten(&SYMBOL_CACHE);
    let gespeichert = kern::geschuetzt(|| SYMBOL_CACHE.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, gr, _)| *k == i && *gr == g).map(|(_, _, b)| b.clone()));
    if let Some(b) = gespeichert {
        return b;
    }
    let b = symbol_entpacken(&erzeugt::SYMBOLE_32, i, 32);
    let b = if g == 32 { b } else { kolibri::libimg::skalieren(&b, g, g, true) };
    // in den gemeinsamen Zwischenspeicher eine eigene Kopie im System-Heap –
    // nur mit Reserve (eine Belegung im Kern-Abschnitt bricht bei leerem
    // System-Heap das ganze System ab; vorher geschah das mit der Meldung)
    let bytes = (g as usize * g as usize).saturating_mul(4);
    if !kern::sys::system_belegung_moeglich(bytes) {
        return b;
    }
    kern::kern_abschnitt(|| {
        let mut c = SYMBOL_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if !c.iter().any(|(k, gr, _)| *k == i && *gr == g) {
            c.push((i, g, b.clone()));
        }
    });
    b
}

/// Bild mit Alpha über einen bekannten Untergrund `hg(x, y)` mischen und
/// zeichnen (Funktion 65 kennt kein Alpha).
pub fn bild_auf(b: &Rgba, x: i32, y: i32, hg: impl Fn(i32, i32) -> u32) {
    let mut m = b.clone();
    for yy in 0..b.h as i32 {
        for xx in 0..b.w as i32 {
            let p = b.px[(yy as u32 * b.w + xx as u32) as usize];
            let a = p >> 24;
            let u = hg(x + xx, y + yy);
            let k = |s: u32| ((p >> s & 255) * a + (u >> s & 255) * (255 - a)) / 255;
            m.px[(yy as u32 * b.w + xx as u32) as usize] = 0xFF00_0000 | k(16) << 16 | k(8) << 8 | k(0);
        }
    }
    kolibri::libimg::zeichnen(&m, x, y, m.w, m.h, 0, 0);
}

fn text(x: i32, y: i32, t: &str, farbe: u32) {
    sys::text((x as u32) << 16 | y as u32 & 0xFFFF, 0x3000_0000 | farbe & 0xFF_FFFF, t.as_bytes(), t.chars().count() as u32, 0);
}

fn text_gross(x: i32, y: i32, t: &str, farbe: u32, faktor: u32) {
    sys::text((x as u32) << 16 | y as u32 & 0xFFFF, 0x3000_0000 | (faktor - 1) << 24 | farbe & 0xFF_FFFF, t.as_bytes(), t.chars().count() as u32, 0);
}

pub fn bildschirm() -> (i32, i32) {
    let g = sys::bildschirmgroesse();
    ((g >> 16) as i32 + 1, (g & 0xFFFF) as i32 + 1)
}

fn farbe(i: usize) -> u32 {
    let mut f = [0u8; 40];
    sys::stil(3, 0, 40, &mut f);
    u32::from_le_bytes([f[4 * i], f[4 * i + 1], f[4 * i + 2], f[4 * i + 3]])
}

/// Spieldaten von Doom/Quake (Shareware von id Software, nicht in ESP32-OS):
/// Doom übernimmt `DOOM1.WAD` beim ersten Start von der SD-Karte auf die
/// interne Partition, Quake liest `pak0.pak` von der SD-Karte.
pub const DOOM_WAD: [&str; 2] = ["/sys/doom/DOOM1.WAD", "/sd0/1/ESP32-OS/doom/DOOM1.WAD"];
pub const QUAKE_PAK: &str = "/sd0/1/ESP32-OS/quake/pak0.pak";

/// Hinweis (Titel, Text), wenn die Spieldaten von `id` fehlen.
pub fn spieldaten_fehlen(id: konsole::SpielId) -> Option<(&'static str, String)> {
    // Entwicklungs-Build (S5-Nachweis): Erststart nachstellen, ohne die
    // vorhandenen Spieldaten anzufassen – gesucht wird in einem leeren
    // Testordner
    let test = option_env!("ESP32OS_SPIELDATEN_TEST").is_some();
    let da = |p: &str| crate::dateimanager::art(&if test { p.replace("/ESP32-OS/", "/ESP32-OS/Spieldaten_Test/").replace("/sys/", "/sys/Spieldaten_Test/") } else { p.to_string() }) == Some(false);
    match id {
        konsole::SpielId::Doom if !DOOM_WAD.iter().any(|p| da(p)) => Some(spieldaten_hinweis(id, false)),
        konsole::SpielId::Quake if !da(QUAKE_PAK) => Some(spieldaten_hinweis(id, false)),
        _ => None,
    }
}

/// NVS-Schlüssel: das Spiel-Programm (Doom/Quake) konnte seine Spieldaten
/// nicht laden (beschädigt, abgeschnitten) und ist in den Desktop zurück –
/// der Desktop zeigt beim Start den Hinweis (`spieldaten_hinweis(id, true)`).
pub const SPIEL_FEHLER_SCHLUESSEL: &str = "spiel_fehler";

/// Hinweis (Titel, Text) zu den Spieldaten von Doom/Quake; `unbrauchbar`:
/// die Datei war da, liess sich aber nicht laden.
pub fn spieldaten_hinweis(id: konsole::SpielId, unbrauchbar: bool) -> (&'static str, String) {
    let (name, datei, ordner) = if id == konsole::SpielId::Quake { ("Quake", "pak0.pak", "quake") } else { ("Doom", "DOOM1.WAD", "doom") };
    if unbrauchbar {
        let text = match kern::sprache::aktuell() {
            kern::sprache::Sprache::Deutsch => format!(
                "{name} konnte {datei} nicht laden (beschädigt oder unvollständig). Shareware von id Software, nicht in ESP32-OS; nur unverändert und kostenlos weitergeben. Datei neu nach ESP32-OS/{ordner}/ auf die SD-Karte legen."
            ),
            kern::sprache::Sprache::Englisch => format!(
                "{name} could not load {datei} (damaged or incomplete). Shareware by id Software, not in ESP32-OS; pass on only unchanged and free of charge. Put the file again in ESP32-OS/{ordner}/ on the SD card."
            ),
        };
        return (kern::sprache::t("Game data unusable", "Spieldaten unbrauchbar"), text);
    }
    let text = match kern::sprache::aktuell() {
        kern::sprache::Sprache::Deutsch => format!(
            "{name} braucht {datei} (Shareware von id Software, nicht in ESP32-OS enthalten; darf nur unverändert und kostenlos weitergegeben werden). Lege die Datei auf die SD-Karte nach ESP32-OS/{ordner}/."
        ),
        kern::sprache::Sprache::Englisch => format!(
            "{name} needs {datei} (shareware by id Software, not included in ESP32-OS; may only be passed on unchanged and free of charge). Put the file on the SD card in ESP32-OS/{ordner}/."
        ),
    };
    (kern::sprache::t("Game data missing", "Spieldaten fehlen"), text)
}

/// Startknopf bei offenem Menü: das Menü schliesst sich (Umschalten wie in
/// anderen Systemen; vorher blieb es offen – Autopilot und Nutzer mussten
/// eine freie Stelle des Desktops treffen).
static MENUE_ZU: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// Zeitpunkt (ms) des letzten Menüstarts (Review Q4: Doppelstart verhindern).
static MENUE_START_MS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Programm starten (wie Funktion 70.7 mit /sys/<name>).
pub fn starten(programm: &str) {
    // Umschalten nur, wenn das Menü noch ein Fenster hat: ein gerade
    // geschlossenes Menü steht kurz noch in der Prozessliste (vorher schloss
    // der nächste Tipp dann ein Menü, das gar nicht mehr offen war)
    let liste = kern::liste();
    let menue_offen = liste.iter().filter(|p| p.name == "@menu").any(|p| sys::fenster_liste().iter().any(|f| f.pid == p.pid));
    let menue_startet = liste.iter().any(|p| p.name == "@menu" && !p.endet);
    let jetzt_ms = (kern::zeit::nanos() / 1_000_000) as u32;
    if programm == "@menu" && menue_offen {
        MENUE_ZU.store(true, core::sync::atomic::Ordering::Relaxed);
        return;
    }
    // Menü gestartet, aber noch ohne Fenster (am Board 100-300 ms):
    // ein zweiter Tipp startet kein zweites Menü
    if programm == "@menu" && menue_startet && jetzt_ms.wrapping_sub(MENUE_START_MS.load(core::sync::atomic::Ordering::Relaxed)) < 700 {
        return;
    }
    if programm == "@menu" {
        MENUE_START_MS.store(jetzt_ms, core::sync::atomic::Ordering::Relaxed);
    }
    match programm {
        p if p.starts_with("@spiel_") && crate::konfig::AUTOPILOT_AKTIV.load(core::sync::atomic::Ordering::Relaxed) => {
            sys::debug_text(&format!("Autopilot: Spielstart {p} unterdrückt"));
        }
        p if p.starts_with("@spiel_") => {
            let id = match &p[7..] {
                "pudding" => konsole::SpielId::Pudding,
                "wuerfelwelt" => konsole::SpielId::Wuerfelwelt,
                "doom" => konsole::SpielId::Doom,
                "quake" => konsole::SpielId::Quake,
                _ => {
                    sys::debug_text(&format!("Start {p}: unbekanntes Spiel"));
                    return;
                }
            };
            // Doom/Quake: Spieldaten sind nicht in ESP32-OS (Auftrag 12.8) –
            // fehlen sie, Hinweis mit Lizenz und Ablageort statt Neustart
            if let Some((titel, text)) = spieldaten_fehlen(id) {
                sys::debug_text(&format!("Spiel {id:?}: Spieldaten fehlen – Hinweis"));
                melden(titel, &text);
                return;
            }
            sys::debug_text(&format!("Spiel starten: {id:?}"));
            let _ = crate::konfig::uhr_sichern();
            if let Some(h) = haken() {
                (h.spiel)(id.als_u8());
            }
        }
        "@spiele" => {
            let _ = kern::datei::starten("/sys/@menu", "spiele");
        }
        "@touchpad" => {
            let _ = crate::konfig::touchpad_setzen(!sys::eingabe::touchpad());
        }
        "@ruhe" => crate::konfig::RUHE_JETZT.store(true, core::sync::atomic::Ordering::Relaxed),
        "@mehr" => {
            let _ = kern::datei::starten("/sys/@menu", "mehr");
        }
        "@system" => {
            let _ = kern::datei::starten("/sys/@menu", "system");
        }
        "@neustart" => {
            let _ = crate::konfig::uhr_sichern();
            if let Some(h) = haken() {
                (h.neustart)();
            }
        }
        p => {
            let r = kern::datei::starten(&format!("/sys/{p}"), "");
            if r < 0 {
                sys::debug_text(&format!("Start {p}: Fehler {}", -r));
                // Q1: sichtbare, verständliche Meldung statt nur im Log
                // (vorher geschah beim Antippen scheinbar nichts)
                let name = anwendungen().into_iter().find(|a| a.programm == p).map_or(p.to_string(), |a| kern::sprache::t(a.titel, a.titel_de).to_string());
                melden(kern::sprache::t("Program not started", "Programm nicht gestartet"), &crate::dateimanager::startfehler(&name, r));
            }
        }
    }
}

// ------------------------------------------------------------ Kontextmenü

/// Kontextmenü (liegt in `kolibri::dialog`, damit auch box_lib es nutzt).
pub use kolibri::dialog::kontextmenue;

// ------------------------------------------------------------ @start

/// Startbild „ESP32-OS“, danach Desktop und Taskleiste.
pub fn start() {
    crate::konfig::laden();
    let (w, h) = bildschirm();
    sys::pruefe_ereignis();
    sys::fenster((w - 1) as u32, (h - 1) as u32, 0x01_000000, 0, None);
    sys::fensterebene(2, u32::MAX, 1);
    let hg = 0x0E1A2B;
    sys::rechteck((w as u32) << 0, h as u32, hg);
    let logo = symbol(49, 128);
    bild_auf(&logo, (w - 128) / 2, h / 2 - 110, |_, _| hg);
    text_gross((w - 8 * 3 * NAME.len() as i32) / 2, h / 2 + 30, NAME, 0xFFFFFF, 3);
    let unter = kern::sprache::t("KolibriOS native on ESP32-P4", "KolibriOS nativ auf ESP32-P4");
    text((w - 8 * unter.chars().count() as i32) / 2, h / 2 + 90, unter, 0x9FB4D0);
    // O7 (Startzeit): Desktop sofort starten, das Startbild bleibt nur, bis
    // die Symbole gezeichnet sind (vorher fest 1,2 s davor), höchstens 3 s
    let t0 = sys::zeit_ns();
    starten("@icon");
    starten("@tastatur");
    // mindestens 0,5 s (das Startbild blitzt sonst nur auf; am Board dauern
    // die Symbole ohnehin länger)
    while (!ICON_GEZEICHNET.load(core::sync::atomic::Ordering::Relaxed) || sys::zeit_ns() - t0 < 500_000_000) && sys::zeit_ns() - t0 < 3_000_000_000 {
        sys::warten(2);
    }
    // Taskleiste erst jetzt (startet sofort; läge sonst über dem Startbild)
    starten("@taskbar");
    sys::debug_text(&format!("Startbild: Desktop bereit nach {} ms", (sys::zeit_ns() - t0) / 1_000_000));
    // Erster Start: Ersteinrichtung (P4)
    if crate::konfig::einrichtung_noetig() {
        sys::debug_text("Ersteinrichtung: startet\n");
        starten("ersteinrichtung");
    } else {
        sys::debug_text("Ersteinrichtung: schon erledigt\n");
    }
}

// ------------------------------------------------------------ @icon

/// Die Symbole des Desktops sind einmal gezeichnet (Startbild darf weg).
static ICON_GEZEICHNET: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Zelle 108 px breit: Beschriftung bis 13 Zeichen („Einstellungen“).
const ZELLE_W: i32 = 108;
const ZELLE_H: i32 = 84;
const SYM: u32 = 48;

/// Senkrechter Verlauf `oben` → `unten` an Zeile `y` von `h`.
fn verlauf(oben: u32, unten: u32, y: i32, h: i32) -> u32 {
    let t = y.clamp(0, h) as u32 * 255 / h.max(1) as u32;
    let mix = |a: u32, b: u32, s: u32| ((a >> s & 255) * (255 - t) + (b >> s & 255) * t) / 255;
    mix(oben, unten, 16) << 16 | mix(oben, unten, 8) << 8 | mix(oben, unten, 0)
}

/// Bild füllend auf w×h skalieren und mittig beschneiden.
pub fn fuellend(b: &Rgba, w: u32, h: u32) -> Rgba {
    let s = (w as f32 / b.w as f32).max(h as f32 / b.h as f32);
    let (sw, sh) = (((b.w as f32 * s).ceil() as u32).max(w), ((b.h as f32 * s).ceil() as u32).max(h));
    let g = kolibri::libimg::skalieren(b, sw, sh, true);
    let (ox, oy) = ((sw - w) / 2, (sh - h) / 2);
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let z = ((y + oy) * g.w + ox) as usize;
        px.extend(g.px[z..z + w as usize].iter().map(|p| p | 0xFF00_0000));
    }
    Rgba { w, h, px, delay: 0 }
}

/// Hintergrund des Desktops: Verlauf (zeilenweise gezeichnet, ohne Puffer)
/// oder das beim Auswählen vorbereitete Bild (`konfig::HG_DATEI`,
/// Bildschirmgrösse). Fehlt das Bild → blauer Verlauf.
pub enum Grund {
    Verlauf(u32, u32),
    Bild(Rgba),
}

pub fn hintergrund_grund(hg: &crate::konfig::Hintergrund, w: i32, h: i32) -> Grund {
    use crate::konfig::{Hintergrund, VERLAEUFE};
    let i = match hg {
        Hintergrund::Bild(p) => match crate::konfig::hintergrund_lesen(w as u32, h as u32) {
            Some(b) => return Grund::Bild(b),
            None => {
                sys::debug_text(&format!("Hintergrund {p}: vorbereitetes Bild fehlt\n"));
                0
            }
        },
        Hintergrund::Verlauf(i) => (*i).min(VERLAEUFE.len() - 1),
    };
    Grund::Verlauf(VERLAEUFE[i].2, VERLAEUFE[i].3)
}

fn icon_zeichnen(w: i32, h: i32, grund: &Grund) {
    use kern::grafik::zu565;
    let t0 = sys::zeit_ns();
    // Ganzer Desktop erst im eigenen Puffer (RGB565 wie der Fensterpuffer),
    // dann in einem Aufruf ins Fenster (O1): jeder Zeichenaufruf wartet sonst
    // auf die Kernsperre, die der Compositor oft hält – am Board 923 ms für
    // Grund, Symbole und Beschriftung. Gleiche Rechnung wie die Aufrufe
    // (Rechteck, Funktion 65 mit 32 bpp, Funktion 4), also gleiche Bildpunkte.
    let (bw, bh) = (w.max(1) as usize, h.max(1) as usize);
    let mut p = kern::grafik::Puffer::neu(bw, bh, 0);
    match grund {
        Grund::Verlauf(oben, unten) => {
            for y in 0..bh {
                let c = zu565(verlauf(*oben, *unten, y as i32, h));
                p.px[y * bw..(y + 1) * bw].fill(c);
            }
        }
        Grund::Bild(b) => {
            for y in 0..bh.min(b.h as usize) {
                for x in 0..bw.min(b.w as usize) {
                    p.px[y * bw + x] = zu565(b.px[y * b.w as usize + x] & 0xFF_FFFF);
                }
            }
        }
    }
    let t_grund = sys::zeit_ns() - t0;
    let (mut t_sym, mut t_misch, mut t_text) = (0u64, 0u64, 0u64);
    let hg_farbe = |x: i32, y: i32| match grund {
        Grund::Verlauf(oben, unten) => verlauf(*oben, *unten, y, h),
        Grund::Bild(b) => b.px[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize] & 0xFF_FFFF,
    };
    let zeilen = ((h - LEISTE_H - 8) / ZELLE_H).max(1);
    let mut knoepfe = Vec::new();
    for (i, a) in anwendungen().iter().filter(|a| a.auf_desktop).enumerate() {
        let (sp, ze) = (i as i32 / zeilen, i as i32 % zeilen);
        let (x, y) = (8 + sp * ZELLE_W, 8 + ze * ZELLE_H);
        knoepfe.push(((x as u32) << 16 | (ZELLE_W - 1) as u32, (y as u32) << 16 | (ZELLE_H - 1) as u32, 0x4000_0000 | (10 + i as u32)));
        let z0 = sys::zeit_ns();
        let s = symbol(a.symbol, SYM);
        let z1 = sys::zeit_ns();
        // Alpha über den bekannten Untergrund mischen (wie bild_auf)
        let (sx, sy) = (x + (ZELLE_W - SYM as i32) / 2, y + 4);
        for yy in 0..s.h as i32 {
            for xx in 0..s.w as i32 {
                let (px, py) = (sx + xx, sy + yy);
                if px < 0 || py < 0 || px >= w || py >= h {
                    continue;
                }
                let q = s.px[(yy as u32 * s.w + xx as u32) as usize];
                let al = q >> 24;
                let u = hg_farbe(px, py);
                let k = |sh: u32| ((q >> sh & 255) * al + (u >> sh & 255) * (255 - al)) / 255;
                p.px[py as usize * bw + px as usize] = zu565(k(16) << 16 | k(8) << 8 | k(0));
            }
        }
        let z2 = sys::zeit_ns();
        // Beschriftung mit Schatten (wie text_schatten, Schrift 8×16 UTF-8)
        let t: String = a.name().chars().take(13).collect();
        let tw = 8 * t.chars().count() as i32;
        let zeichen = kern::schrift::zeichen(t.as_bytes(), kern::schrift::Kodierung::Utf8, false, t.chars().count());
        let mut z = kern::grafik::Zeichner::neu(&mut p);
        for (dx, farbe) in [(1, 0x101010), (0, 0xFF_FFFF)] {
            let c = zu565(farbe);
            kern::schrift::zeichnen(
                &zeichen,
                kern::schrift::Kodierung::Utf8,
                1,
                x + (ZELLE_W - tw) / 2 + dx,
                y + 8 + SYM as i32 + dx,
                &mut |rx, ry, rw, rh| z.fuellen565(rx, ry, rw, rh, c),
                None,
            );
        }
        t_sym += z1 - z0;
        t_misch += z2 - z1;
        t_text += sys::zeit_ns() - z2;
    }
    let z_aus = sys::zeit_ns();
    let roh: Vec<u8> = p.px.iter().flat_map(|v| v.to_le_bytes()).collect();
    sys::neuzeichnen(1);
    sys::fenster((w - 1) as u32, (h - 1) as u32, 0x41_000000, 0, None);
    sys::palettenbild(&roh, (bw as u32) << 16 | bh as u32, 0, 16, &[], 0);
    for (kx, ky, id) in knoepfe {
        sys::knopf(kx, ky, id, 0);
    }
    sys::neuzeichnen(2);
    ICON_GEZEICHNET.store(true, core::sync::atomic::Ordering::Relaxed);
    sys::debug_text(&format!(
        "@icon: gezeichnet in {} ms (Grund {}, Symbole {}, Mischen {}, Text {}, Ausgabe {})\n",
        (sys::zeit_ns() - t0) / 1_000_000,
        t_grund / 1_000_000,
        t_sym / 1_000_000,
        t_misch / 1_000_000,
        t_text / 1_000_000,
        (sys::zeit_ns() - z_aus) / 1_000_000
    ));
}

/// Bildschirm-Mitte des Desktop-Symbols von `programm` – für Tests und den
/// Autopiloten (Raster wie beim Zeichnen: spaltenweise, 108×84).
pub fn symbol_mitte(programm: &str) -> Option<(i32, i32)> {
    let (_, h) = bildschirm();
    let zeilen = ((h - LEISTE_H - 8) / ZELLE_H).max(1);
    let i = anwendungen().iter().filter(|a| a.auf_desktop).position(|a| a.programm == programm)? as i32;
    let (sp, ze) = (i / zeilen, i % zeilen);
    Some((8 + sp * ZELLE_W + ZELLE_W / 2, 8 + ze * ZELLE_H + ZELLE_H / 2))
}

/// Desktop: Hintergrund und Symbole (unterste Ebene).
pub fn icon() {
    let (w, h) = bildschirm();
    sys::maske(0x05);
    let mut erst = true;
    // Hintergrund nur neu berechnen, wenn er sich geändert hat
    let mut bild: Option<(crate::konfig::Hintergrund, Grund)> = None;
    loop {
        match sys::warte_ereignis() {
            1 => {
                let hg = crate::konfig::hintergrund();
                if bild.as_ref().is_none_or(|(k, _)| *k != hg) {
                    let b = hintergrund_grund(&hg, w, h);
                    bild = Some((hg, b));
                }
                if let Some((_, b)) = &bild {
                    icon_zeichnen(w, h, b);
                }
                if erst {
                    sys::fensterebene(2, u32::MAX, (-2i32) as u32);
                    erst = false;
                }
            }
            3 => {
                let k = sys::knopf_id();
                let (id, rechts) = (k >> 8, k & 2 != 0);
                let liste: Vec<Anwendung> = anwendungen().into_iter().filter(|a| a.auf_desktop).collect();
                if let Some(a) = id.checked_sub(10).and_then(|i| liste.get(i as usize)) {
                    if rechts {
                        // Langer Druck (Rechtsklick): Kontextmenü
                        let m = sys::maus(0);
                        let wahl = kontextmenue((m >> 16) as i32, (m & 0xFFFF) as i32, &[kern::sprache::t("Open", "Öffnen"), kern::sprache::t("Cancel", "Abbrechen")]);
                        if wahl == Some(0) {
                            starten(a.programm);
                        }
                    } else {
                        starten(a.programm);
                    }
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------ @taskbar

const START_W: i32 = 112;
const FENSTER_W: i32 = 132;
const KNOPF_START: u32 = 2;
const KNOPF_TASTATUR: u32 = 3;
/// Uhr: langer Druck = Bildschirmfoto (P6).
const KNOPF_UHR: u32 = 4;
/// WLAN-Anzeige (W6): öffnet Einstellungen → WLAN.
const KNOPF_WLAN: u32 = 5;
/// Breite der WLAN-Anzeige.
const WLAN_W: i32 = 36;

/// WLAN-Zustand für die Taskleiste: 0 = kein WLAN, sonst Bit 0 = vorhanden,
/// 1 = verbunden, 2 = verbindet, 3..5 = Balken.
pub fn wlan_stand() -> u32 {
    if !kern::wlan::vorhanden() {
        return 0;
    }
    let z = kern::wlan::zustand();
    1 | u32::from(z.verbunden) << 1 | u32::from(z.verbindet) << 2 | u32::from(kern::wlan::balken(z.rssi)) << 3
}
const KNOPF_FENSTER: u32 = 100;

fn leiste_zeichnen(w: i32, y0: i32, eigen: u32) -> Vec<kern::fenster::FensterInfo> {
    let bg = farbe(0); // taskbar (dtp)
    let tf = farbe(1); // taskbar_text
    sys::neuzeichnen(1);
    sys::fenster((w - 1) as u32, (y0 as u32) << 16 | (LEISTE_H - 1) as u32, 0x01_000000, 0, None);
    sys::rechteck(w as u32, LEISTE_H as u32, bg);
    sys::rechteck(w as u32, 1, 0xFFFFFF & (bg | 0x303030));
    // Start
    sys::knopf(2 << 16 | (START_W - 4) as u32, 3 << 16 | (LEISTE_H - 7) as u32, KNOPF_START, 0x3A7AC0);
    bild_auf(&symbol(49, 28), 8, 6, |_, _| 0x3A7AC0);
    text(40, (LEISTE_H - 16) / 2, NAME, 0xFFFFFF);
    // Fenster der Programme
    // ohne Dienste, deren Fenster noch den Programmnamen tragen („@icon“,
    // bevor es auf die unterste Ebene geht – sonst beim Start kurz sichtbar)
    let liste: Vec<_> = sys::fenster_liste().into_iter().filter(|f| f.ebene == 0 && f.pid != eigen && !f.titel.starts_with('@')).collect();
    // Akkustand (nur wenn die Firmware misst, siehe board.rs) links neben
    // dem Tastaturknopf
    let akku = crate::konfig::AKKU.load(core::sync::atomic::Ordering::Relaxed);
    // WLAN-Anzeige (W6) links davon, wenn es WLAN gibt
    let wlan = wlan_stand();
    let rechts = 64 + 48 + if akku <= 100 { 56 } else { 0 } + if wlan != 0 { WLAN_W } else { 0 };
    let platz = ((w - START_W - rechts) / FENSTER_W).max(0) as usize;
    for (i, f) in liste.iter().take(platz).enumerate() {
        let x = START_W + 4 + i as i32 * FENSTER_W;
        let c = if f.aktiv { 0x7887A6 } else if f.minimiert { 0x3E4A63 } else { 0x4E5A75 };
        sys::knopf((x as u32) << 16 | (FENSTER_W - 6) as u32, 3 << 16 | (LEISTE_H - 7) as u32, KNOPF_FENSTER + i as u32, c);
        // Q1: zu lange Titel mit „..“ statt hart abgeschnitten
        let max = ((FENSTER_W - 14) / 8) as usize;
        let t: String = if f.titel.chars().count() <= max { f.titel.clone() } else { f.titel.chars().take(max - 2).collect::<String>() + ".." };
        text(x + 6, (LEISTE_H - 16) / 2, &t, tf);
    }
    if wlan != 0 {
        let x = w - rechts + 2;
        sys::knopf((x as u32) << 16 | (WLAN_W - 4) as u32, 3 << 16 | (LEISTE_H - 7) as u32, KNOPF_WLAN | 0x4000_0000, 0);
        let (verbunden, verbindet, balken) = (wlan & 2 != 0, wlan & 4 != 0, (wlan >> 3 & 7) as u8);
        for i in 0..4u8 {
            let bh = 6 + i32::from(i) * 4;
            let c = if verbunden && i < balken {
                0xFFFFFF
            } else if verbindet {
                0xC8B060
            } else {
                0x6A7488
            };
            sys::rechteck(((x + 6 + i32::from(i) * 6) as u32) << 16 | 4, ((32 - bh) as u32) << 16 | bh as u32, c);
        }
        if !verbunden && !verbindet {
            // getrennt: kleines rotes Kreuz
            for d in 0..7 {
                sys::rechteck(((x + 22 + d) as u32) << 16 | 1, ((10 + d) as u32) << 16 | 1, 0xE04040);
                sys::rechteck(((x + 28 - d) as u32) << 16 | 1, ((10 + d) as u32) << 16 | 1, 0xE04040);
            }
        }
    }
    if akku <= 100 {
        let ax = w - rechts + 4 + if wlan != 0 { WLAN_W } else { 0 };
        let c = if akku < 20 { 0xE04040 } else { 0x60D070 };
        sys::rechteck((ax as u32) << 16 | 22, 16 << 16 | 14, 0xD0D0D0);
        sys::rechteck(((ax + 22) as u32) << 16 | 2, 20 << 16 | 6, 0xD0D0D0);
        sys::rechteck(((ax + 1) as u32) << 16 | 20, 17 << 16 | 12, bg);
        sys::rechteck(((ax + 2) as u32) << 16 | (u32::from(akku) * 18 / 100).max(1), 18 << 16 | 10, c);
        let p = format!("{akku}%");
        text(ax + 28, (LEISTE_H - 16) / 2, &p, tf);
    }
    // Bildschirmtastatur ein/aus
    let kx = w - rechts + 2 + if akku <= 100 { 56 } else { 0 } + if wlan != 0 { WLAN_W } else { 0 };
    sys::knopf((kx as u32) << 16 | 42, 3 << 16 | (LEISTE_H - 7) as u32, KNOPF_TASTATUR, 0x4E5A75);
    bild_auf(&symbol(118, 28), kx + 8, 6, |_, _| 0x4E5A75);
    // Uhr (unsichtbarer Knopf: langer Druck = Bildschirmfoto)
    sys::knopf(((w - 8 * 5 - 16) as u32) << 16 | (8 * 5 + 14) as u32, 3 << 16 | (LEISTE_H - 7) as u32, KNOPF_UHR | 0x4000_0000, 0);
    let z = sys::uhrzeit();
    let bcd = |v: u32| (v >> 4) * 10 + (v & 15);
    let uhr = format!("{:02}:{:02}", bcd(z & 0xFF), bcd(z >> 8 & 0xFF));
    text(w - 8 * 5 - 12, (LEISTE_H - 16) / 2, &uhr, tf);
    sys::neuzeichnen(2);
    liste
}

/// Taskleiste: immer oben, Arbeitsfläche ohne Leiste.
pub fn taskbar() {
    let (w, h) = bildschirm();
    let y0 = h - LEISTE_H;
    let eigen = kern::eigene_pid().unwrap_or(0);
    sys::maske(0x07);
    // Arbeitsfläche (48.6): maximierte Fenster lassen die Leiste frei
    sys::stil(6, (w - 1) as u32, (y0 - 1) as u32, &mut []);
    let mut liste = Vec::new();
    let mut erst = true;
    let mut letzte = (u32::MAX, 0usize);
    // S1: Änderungen der Fensterliste sofort (Ereignis statt Abfrage)
    sys::fensterliste_beobachten();
    loop {
        let e = sys::warte_ereignis_max(50);
        // Wecker der Uhr (A3) – klingelt auch, wenn die Uhr nicht offen ist
        crate::uhr::wecker_pruefen();
        // Uhrzeit 0x00SSMMHH: nur Stunden und Minuten zählen
        let stand = (
            sys::uhrzeit() & 0xFFFF | u32::from(crate::konfig::AKKU.load(core::sync::atomic::Ordering::Relaxed)) << 16 | wlan_stand() << 24,
            sys::fenster_liste().iter().map(|f| f.slot * 7 + f.aktiv as usize * 3 + f.minimiert as usize).sum(),
        );
        match e {
            1 => {
                liste = leiste_zeichnen(w, y0, eigen);
                // S1-Messung: Eintrag gezeichnet – wie lange nach der
                // Änderung der Fensterliste (Log, am Board ausgewertet)
                let m = kern::fenster::LISTE_GEAENDERT_MS.swap(0, core::sync::atomic::Ordering::Relaxed);
                if m != 0 && !erst {
                    let jetzt = (sys::zeit_ns() / 1_000_000) as u32 | 1;
                    sys::debug_text(&format!("Taskleiste: Eintrag {} ms nach der Fensteränderung\n", jetzt.wrapping_sub(m)));
                }
                if erst {
                    sys::fensterebene(2, u32::MAX, 1);
                    erst = false;
                }
            }
            3 => {
                let k = sys::knopf_id();
                let (id, rechts) = (k >> 8, k & 2 != 0);
                match id {
                    KNOPF_START => starten("@menu"),
                    KNOPF_TASTATUR => sys::tastatur_umschalten(),
                    // langer Druck auf die Uhr = Bildschirmfoto
                    KNOPF_UHR if rechts => starten("@screenshot"),
                    KNOPF_UHR => {}
                    KNOPF_WLAN => {
                        let _ = kern::datei::starten("/sys/einstellungen", "wifi");
                    }
                    id if id >= KNOPF_FENSTER => {
                        if let Some(f) = liste.get((id - KNOPF_FENSTER) as usize) {
                            // aktiv → minimieren, sonst nach vorne
                            sys::minimieren(if f.aktiv { 0 } else { 2 }, f.slot as u32);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        if stand != letzte {
            letzte = stand;
            liste = leiste_zeichnen(w, y0, eigen);
        }
    }
}

// ------------------------------------------------------------ @menu

const ZEILE_M: i32 = 44;

/// Einträge des Startmenüs (Name, Programm, Symbol). `ordner`: "" = das
/// Hauptmenü, "mehr" = Programme ohne Desktop-Symbol (Beispiele), "system" =
/// Task-Manager, Bildschirmfoto, Touchpad-Modus, "spiele" = die Spiele. Das
/// Hauptmenü bleibt so bei höchstens 15 Einträgen (3 Spalten × 5 Zeilen über
/// der Taskleiste).
fn menue_eintraege(ordner: &str) -> Vec<(String, &'static str, usize)> {
    let alle = anwendungen();
    match ordner {
        "mehr" => alle.iter().filter(|a| !a.auf_desktop).map(|a| (a.name().to_string(), a.programm, a.symbol)).collect(),
        "spiele" => alle.iter().filter(|a| a.programm.starts_with("@spiel_")).map(|a| (a.name().to_string(), a.programm, a.symbol)).collect(),
        "system" => vec![
            (kern::sprache::t("Task manager", "Task-Manager").into(), "taskmanager", 57),
            (kern::sprache::t("Screenshot", "Bildschirmfoto").into(), "@screenshot", 70),
            (if sys::eingabe::touchpad() { kern::sprache::t("Touchpad mode off", "Touchpad-Modus aus") } else { kern::sprache::t("Touchpad mode on", "Touchpad-Modus ein") }.into(), "@touchpad", 54),
        ],
        _ => {
            let mut eintraege: Vec<(String, &'static str, usize)> =
                alle.iter().filter(|a| a.auf_desktop && !a.programm.starts_with("@spiel_")).map(|a| (a.name().to_string(), a.programm, a.symbol)).collect();
            if alle.iter().any(|a| a.programm.starts_with("@spiel_")) {
                eintraege.push((kern::sprache::t("Games...", "Spiele...").into(), "@spiele", 35));
            }
            if !eintraege.iter().any(|e| e.1 == "info") {
                eintraege.push((format!("{} {NAME}", kern::sprache::t("About", "Über")), "info", 113));
            }
            if alle.iter().any(|a| !a.auf_desktop) {
                eintraege.push((kern::sprache::t("More programs...", "Mehr Programme...").into(), "@mehr", 2));
            }
            eintraege.push((kern::sprache::t("System...", "System...").into(), "@system", 6));
            eintraege.push((kern::sprache::t("Sleep", "Ruhezustand").into(), "@ruhe", 76));
            eintraege.push((kern::sprache::t("Restart", "Neustart").into(), "@neustart", 12));
            eintraege
        }
    }
}

/// Aufbau des Startmenüs: (Zeilen, Spaltenbreite, Breite, Höhe, y des
/// Fensters). Bis 6 Einträge eine Spalte, bis 10 zwei, darüber drei
/// schmalere (höchstens 5 Zeilen passen über die Taskleiste).
fn menue_aufbau(n: usize, skin: i32) -> (i32, i32, i32, i32, i32) {
    let (_, h) = bildschirm();
    let spalten = match n {
        0..=6 => 1,
        7..=10 => 2,
        _ => 3,
    };
    let sw = if spalten == 3 { 210 } else { 230 };
    let zeilen = n.div_ceil(spalten) as i32;
    let mw = sw * spalten as i32 + 10;
    let mh = (skin + 5 + zeilen * ZEILE_M + 10).min(h - LEISTE_H);
    (zeilen, sw, mw, mh, h - LEISTE_H - mh)
}

/// Bildschirm-Mitte des Eintrags `programm` im Hauptmenü bzw. im Ordner
/// (`menue_eintrag_mitte_in("system", …)`) – für Tests und den Autopiloten,
/// damit neue Programme sie nicht verschieben.
pub fn menue_eintrag_mitte(programm: &str) -> Option<(i32, i32)> {
    menue_eintrag_mitte_in("", programm)
}

/// Wie [`menue_eintrag_mitte`], im Ordner `ordner` ("", "mehr", "system",
/// "spiele").
pub fn menue_eintrag_mitte_in(ordner: &str, programm: &str) -> Option<(i32, i32)> {
    let eintraege = menue_eintraege(ordner);
    let i = eintraege.iter().position(|e| e.1 == programm)? as i32;
    let skin = sys::stil(4, 0, 0, &mut []).0 as i32;
    let (zeilen, sw, _, _, my) = menue_aufbau(eintraege.len(), skin);
    let (sp, ze) = (i / zeilen, i % zeilen);
    Some((5 + 2 + sp * sw + (sw - 5) / 2, my + skin + 2 + ze * ZEILE_M + (ZEILE_M - 3) / 2))
}

/// Startmenü: Programme mit Symbol, „Über ESP32-OS“, Spielekonsole, Neustart.
pub fn menu() {
    // Parameter: Ordner („mehr“: Programme ohne Desktop-Symbol, „system“,
    // „spiele“)
    let eintraege = menue_eintraege(&kern::argumente());
    let skin = sys::stil(4, 0, 0, &mut []).0 as i32;
    let (zeilen, sw, mw, mh, my) = menue_aufbau(eintraege.len(), skin);
    sys::maske(0x07);
    let zeichnen = || {
        sys::neuzeichnen(1);
        sys::fenster((mw - 1) as u32, (my as u32) << 16 | (mh - 1) as u32, 0x33 << 24 | 0xF4F4F4, 0, Some(NAME));
        for (i, (t, _, s)) in eintraege.iter().enumerate() {
            let (sp, ze) = (i as i32 / zeilen, i as i32 % zeilen);
            let (x, y) = (2 + sp * sw, 2 + ze * ZEILE_M);
            sys::knopf((x as u32) << 16 | (sw - 5) as u32, (y as u32) << 16 | (ZEILE_M - 3) as u32, 10 + i as u32, 0xF4F4F4);
            bild_auf(&symbol(*s, 32), x + 6, y + (ZEILE_M - 32) / 2 - 1, |_, _| 0xF4F4F4);
            text(x + 46, y + (ZEILE_M - 16) / 2 - 1, t, 0x101010);
        }
        sys::neuzeichnen(2);
    };
    let eigen = kern::eigene_tid();
    let mut gezeigt = false;
    MENUE_ZU.store(false, core::sync::atomic::Ordering::Relaxed);
    loop {
        // Startknopf noch einmal gedrückt: zu
        if MENUE_ZU.swap(false, core::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match sys::warte_ereignis_max(10) {
            1 => {
                zeichnen();
                gezeigt = true;
            }
            3 => {
                let id = sys::knopf_id() >> 8;
                if id == 1 {
                    return;
                }
                if let Some((_, p, _)) = id.checked_sub(10).and_then(|i| eintraege.get(i as usize)) {
                    starten(p);
                    return;
                }
            }
            _ => {}
        }
        // Menü schliesst, sobald etwas anderes angetippt wird
        let aktiv = sys::system(7, 0);
        if gezeigt && eigen.is_some_and(|t| sys::system(21, t) != aktiv) && sys::fenster_liste().iter().any(|f| f.aktiv) {
            return;
        }
    }
}

// ------------------------------------------------------------ @meldung

/// Meldungsdialog starten (Titel, Text); eigenes Programm, damit er auch
/// erscheint, wenn der Absender nicht mehr läuft.
pub fn melden(titel: &str, text: &str) {
    let a: String = format!("{titel}\n{text}").chars().take(250).collect();
    let r = kern::datei::starten("/sys/@meldung", &a);
    if r < 0 {
        sys::debug_text(&format!("Meldung: Fehler {}", -r));
    }
}

/// Meldungsdialog (Parameter: „Titel\nText“), OK per Touch.
pub fn meldung() {
    let a = kern::argumente();
    let (titel, inhalt) = a.split_once('\n').unwrap_or((kern::sprache::t("Message", "Meldung"), a.as_str()));
    let (titel, inhalt) = (titel.to_string(), inhalt.to_string());
    // Text auf Zeilen zu höchstens 38 Zeichen umbrechen (320 px bei 8×16)
    let mut zeilen: Vec<String> = Vec::new();
    for absatz in inhalt.split('\n') {
        let mut z = String::new();
        for wort in absatz.split(' ') {
            if z.chars().count() + wort.chars().count() + 1 > 38 && !z.is_empty() {
                zeilen.push(std::mem::take(&mut z));
            }
            if !z.is_empty() {
                z.push(' ');
            }
            z.push_str(wort);
        }
        zeilen.push(z);
    }
    let (w, h) = (400, 140 + zeilen.len() as i32 * 20);
    let (sw, sh) = bildschirm();
    let (x, y) = ((sw - w) / 2, ((sh - LEISTE_H - h) / 2).max(0));
    loop {
        match sys::warte_ereignis() {
            1 => {
                sys::neuzeichnen(1);
                sys::fenster((x as u32) << 16 | (w - 1) as u32, (y as u32) << 16 | (h - 1) as u32, 0x33 << 24 | 0xF4F4F4, 0, Some(&titel));
                bild_auf(&symbol(14, 48), 10, 10, |_, _| 0xF4F4F4);
                for (i, z) in zeilen.iter().enumerate() {
                    text(70, 14 + i as i32 * 20, z, 0x202020);
                }
                let by = 14 + zeilen.len() as i32 * 20 + 14;
                sys::knopf(((w - 10 - 110 - 10) as u32) << 16 | 109, (by as u32) << 16 | 39, 2, 0x3A7AC0);
                text(w - 10 - 110 - 10 + 47, by + 12, "OK", 0xFFFFFF);
                sys::neuzeichnen(2);
            }
            3 => {
                let id = sys::knopf_id() >> 8;
                if id == 1 || id == 2 {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Systemdienste, die nach einem Absturz neu gestartet werden.
pub const DIENSTE: [&str; 3] = ["@icon", "@taskbar", "@tastatur"];

/// Wächter (läuft in der Anzeigeschleife, nicht als Programm – so überlebt
/// er jeden Programmabsturz): meldet Abstürze aller Programme und startet
/// abgestürzte Systemdienste neu.
pub fn waechter() {
    for (name, ende) in kern::neue_abstuerze() {
        let grund = match ende {
            kern::Ende::Absturz(m) => m.lines().last().unwrap_or("Panik").to_string(),
            kern::Ende::SpeicherVoll => kern::sprache::t("program memory full", "Speicher des Programms voll").to_string(),
            _ => continue,
        };
        if name != "@meldung" {
            absturz_melden(name, &grund);
        }
        if DIENSTE.contains(&name) {
            let _ = kern::starte_name(name);
        }
    }
}

/// Absturz melden (Rust-Panik/Speicher voll ohne Neustart, oder nach dem
/// Neustart nach einer CPU-Ausnahme).
pub fn absturz_melden(programm: &str, grund: &str) {
    let text = match kern::sprache::aktuell() {
        kern::sprache::Sprache::Englisch => format!("The program \"{programm}\" crashed and was closed.\nReason: {grund}"),
        kern::sprache::Sprache::Deutsch => format!("Das Programm \"{programm}\" ist abgestürzt und wurde beendet.\nGrund: {grund}"),
    };
    melden(kern::sprache::t("Program crashed", "Programm abgestürzt"), &text);
}

// ------------------------------------------------------------ info

#[cfg(not(feature = "guition"))]
const BOARD_NAME: &str = "Waveshare ESP32-P4-WIFI6-Touch-LCD-5";
#[cfg(feature = "guition")]
const BOARD_NAME: &str = "Guition JC4880P443C_I_W";

/// Info-Dialog „Über ESP32-OS“.
pub fn info() {
    let programme = kern::liste().len();
    let zeilen = [
        format!("Version {VERSION}"),
        kern::sprache::t("KolibriOS system functions native,", "KolibriOS-Systemfunktionen nativ,").to_string(),
        kern::sprache::t("without emulator", "ohne Emulator").to_string(),
        BOARD_NAME.to_string(),
        format!("{} {programme}", kern::sprache::t("Running programs:", "Laufende Programme:")),
        kern::sprache::t("KolibriOS parts: GPL-2.0 (LICENSE)", "KolibriOS-Teile: GPL-2.0 (LICENSE)").to_string(),
    ];
    let (w, h) = (440, 280);
    loop {
        match sys::warte_ereignis() {
            1 => {
                sys::neuzeichnen(1);
                sys::fenster(100 << 16 | (w - 1), 20 << 16 | (h - 1), 0x33 << 24 | 0xF4F4F4, 0, Some(&format!("{} {NAME}", kern::sprache::t("About", "Über"))));
                bild_auf(&symbol(49, 64), 10, 10, |_, _| 0xF4F4F4);
                text_gross(90, 18, NAME, 0x1A3A6A, 2);
                for (i, z) in zeilen.iter().enumerate() {
                    text(90, 58 + i as i32 * 20, z, 0x202020);
                }
                sys::knopf(320 << 16 | 99, 182 << 16 | 39, 2, 0x3A7AC0);
                text(320 + 42, 182 + 12, "OK", 0xFFFFFF);
                sys::neuzeichnen(2);
            }
            3 => {
                let id = sys::knopf_id() >> 8;
                if id == 1 || id == 2 {
                    return;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O1: Die beim Bauen zerlegten Symbole stimmen mit dem Streifen überein.
    #[test]
    fn symbole_wie_streifen() {
        let s = kolibri::dekoder::dekodieren(include_bytes!("../daten/icons32.png")).expect("PNG").1.pop().expect("Bild");
        assert_eq!(s.h as usize / 32, erzeugt::SYMBOLE_ANZAHL);
        for i in 0..erzeugt::SYMBOLE_ANZAHL {
            let k = symbol_entpacken(&erzeugt::SYMBOLE_32, i, 32);
            assert_eq!(k.px, s.px[i * 1024..(i + 1) * 1024], "Symbol {i} 32×32");
        }
    }
}
