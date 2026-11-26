//! WLAN (Block D) über den ESP32-C6: esp_hosted (SDIO) + esp_wifi_remote,
//! angesteuert über `komponenten/esp32os/wlan.c`. Der Treiber [`C6`] wird
//! beim Start bei `kern::wlan` angemeldet; ein eigener Faden führt
//! `kern::wlan::wache` aus (Zustand, automatisch verbinden).
//!
//! Bekannte Netze liegen im NVS (Namensraum „pudding“, Schlüssel
//! `wlan_netze`). Zugangsdaten erscheinen nie im Log. Test-Zugangsdaten
//! kommen nur aus der lokalen, von Git ignorierten Datei `wlan.local`
//! (Build-Variablen, `bauen_flashen.ps1 -Wlan`).

use core::ffi::{c_char, c_int, c_uint};
use konsole::start::Speicher;
use std::ffi::CString;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct ZustandRoh {
    bereit: c_int,
    verbindet: c_int,
    verbunden: c_int,
    /// IPv4 in Netzwerk-Bytefolge.
    ip: c_uint,
    rssi: c_int,
    kanal: c_int,
    /// Bit 0 = b, 1 = g, 2 = n, 3 = ax; Bit 7 = ausgehandelt (sonst:
    /// Fähigkeiten des Zugangspunkts, alte C6-Firmware).
    phy: c_int,
    grund: c_int,
    /// Netzmaske, Gateway, DNS (Netzwerk-Bytefolge).
    maske: c_uint,
    gateway: c_uint,
    dns: c_uint,
    mac: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NetzRoh {
    ssid: [c_char; 33],
    rssi: c_int,
    kanal: c_int,
    sicherheit: c_int,
    ax: c_int,
}

unsafe extern "C" {
    fn esp32os_wlan_start() -> c_int;
    fn esp32os_wlan_verbinden(ssid: *const c_char, passwort: *const c_char) -> c_int;
    fn esp32os_wlan_trennen() -> c_int;
    fn esp32os_wlan_zustand(z: *mut ZustandRoh) -> c_int;
    fn esp32os_wlan_suchen(liste: *mut NetzRoh, max: c_int) -> c_int;
    fn esp32os_wlan_versionen(host: *mut u32, cp: *mut u32, cp_ziel: *mut c_char, ziel_len: c_int) -> c_int;
    #[cfg(esp32os_c6_ota)]
    fn esp32os_c6_ota(bild: *const u8, n: u32, alte_firmware: c_int) -> c_int;
    fn esp32os_wlan_sparen(an: c_int) -> c_int;
    fn esp32os_ntp_start() -> c_int;
    fn esp32os_ntp_stopp();
    fn esp32os_ntp_gestellt() -> c_int;
}

/// Versionen von esp_hosted (Host) und C6-Firmware als (Haupt, Neben, Fix).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Versionen {
    pub host: (u8, u8, u8),
    /// `None`, wenn der C6 die Abfrage nicht beantwortet.
    pub cp: Option<(u8, u8, u8)>,
    pub cp_ziel: String,
}

fn drei(v: u32) -> (u8, u8, u8) {
    ((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Versionen abfragen (nach dem Start).
pub fn versionen() -> Versionen {
    let (mut h, mut c) = (0u32, 0u32);
    let mut ziel = [0 as c_char; 16];
    // SAFETY: Ausgaben in lokale Variablen, Puffergröße übergeben.
    let r = unsafe { esp32os_wlan_versionen(&mut h, &mut c, ziel.as_mut_ptr(), ziel.len() as c_int) };
    let b: Vec<u8> = ziel.iter().take_while(|&&x| x != 0).map(|&x| x as u8).collect();
    Versionen { host: drei(h), cp: (r == 0).then(|| drei(c)), cp_ziel: String::from_utf8_lossy(&b).into_owned() }
}

/// C2 (W2), Entwicklungs-Build `ESP32OS_C6_OTA` (Board-Test 6, nur mit OK
/// des Nutzers): die eigene C6-Firmware (werkzeuge/c6_bauen.ps1) vom P4 aus
/// auf den C6 schreiben – einmal je Build (NVS-Merker), danach Neustart des
/// Boards (der P4 setzt den C6 beim Start zurück, er startet die neue
/// Firmware). Rückweg: die C6-Firmware schaltet selbst auf die alte zurück,
/// wenn sich der P4 in der Probezeit nicht meldet (c6_firmware/main/main.c).
#[cfg(esp32os_c6_ota)]
fn c6_aktualisieren(alte_firmware: bool) {
    static BILD: &[u8] = include_bytes!(env!("ESP32OS_C6_BILD"));
    const MERKER: &str = "c6_ota";
    let Ok(n) = crate::store::Nvs::open() else {
        log::error!("C6-Update: NVS nicht offen – kein Update");
        return;
    };
    if n.lesen(MERKER).is_some() {
        log::warn!("C6-Update: schon einmal versucht (NVS „{MERKER}“) – kein zweites Mal");
        return;
    }
    n.schreiben(MERKER, &[1]);
    let laenge = BILD.len() as u32;
    let _ = std::thread::Builder::new().name("c6_ota".into()).stack_size(8192).spawn(move || {
        // SAFETY: Bild ist statisch, Länge passt.
        let r = unsafe { esp32os_c6_ota(BILD.as_ptr(), laenge, c_int::from(alte_firmware)) };
        if r == 0 {
            log::warn!("C6-Update: fertig – Neustart in 3 s");
            std::thread::sleep(std::time::Duration::from_secs(3));
            // SAFETY: Neustart des Boards (setzt auch den C6 zurück).
            unsafe { esp_idf_svc::sys::esp_restart() };
        } else {
            log::error!("C6-Update: gescheitert ({r}) – alte C6-Firmware bleibt aktiv");
        }
    });
}

fn ip_text(ip: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

const NVS_SCHLUESSEL: &str = "wlan_netze";
const NVS_FORMAT: u8 = 1;

/// Bekannte Netze → NVS-Blob: Format-Byte, dann je Netz Länge + SSID,
/// Länge + Passwort.
pub fn netze_kodieren(netze: &[(String, String)]) -> Vec<u8> {
    let mut b = vec![NVS_FORMAT];
    for (s, p) in netze {
        for t in [s, p] {
            let t = &t.as_bytes()[..t.len().min(64)];
            b.push(t.len() as u8);
            b.extend_from_slice(t);
        }
    }
    b
}

/// NVS-Blob → bekannte Netze (unbekanntes Format oder abgeschnitten: leer
/// bzw. bis zum letzten vollständigen Eintrag).
pub fn netze_dekodieren(b: &[u8]) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if b.first() != Some(&NVS_FORMAT) {
        return v;
    }
    let feld = |i: &mut usize| -> Option<String> {
        let n = usize::from(*b.get(*i)?);
        let t = b.get(*i + 1..*i + 1 + n)?;
        *i += 1 + n;
        Some(String::from_utf8_lossy(t).into_owned())
    };
    let mut i = 1;
    while i < b.len() {
        let (Some(s), Some(p)) = (feld(&mut i), feld(&mut i)) else { break };
        v.push((s, p));
    }
    v
}

/// Treiber für `kern::wlan`.
pub struct C6;

impl kern::wlan::Treiber for C6 {
    fn zustand(&self) -> kern::wlan::Zustand {
        let mut z = ZustandRoh::default();
        // SAFETY: Ausgabe in lokale Struktur.
        unsafe { esp32os_wlan_zustand(&mut z) };
        kern::wlan::Zustand {
            bereit: z.bereit != 0,
            verbindet: z.verbindet != 0,
            verbunden: z.verbunden != 0,
            ip: z.ip.to_le_bytes(),
            rssi: z.rssi,
            kanal: z.kanal.clamp(0, 255) as u8,
            he: z.phy & 8 != 0,
            grund: z.grund.clamp(0, 0xFFFF) as u16,
            maske: z.maske.to_le_bytes(),
            gateway: z.gateway.to_le_bytes(),
            dns: z.dns.to_le_bytes(),
            mac: z.mac,
        }
    }

    fn suchen(&self) -> Result<Vec<kern::wlan::Netz>, i32> {
        let mut roh = vec![NetzRoh { ssid: [0; 33], rssi: 0, kanal: 0, sicherheit: 0, ax: 0 }; 32];
        // SAFETY: Puffer für 32 Einträge.
        let n = unsafe { esp32os_wlan_suchen(roh.as_mut_ptr(), roh.len() as c_int) };
        if n < 0 {
            return Err(-n);
        }
        Ok(roh[..n as usize]
            .iter()
            .map(|r| {
                let b: Vec<u8> = r.ssid.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
                kern::wlan::Netz {
                    ssid: String::from_utf8_lossy(&b).into_owned(),
                    rssi: r.rssi,
                    kanal: r.kanal.clamp(0, 255) as u8,
                    sicherheit: kern::wlan::Sicherheit::aus_authmode(r.sicherheit),
                    ax: r.ax != 0,
                }
            })
            .collect())
    }

    fn verbinden(&self, ssid: &str, passwort: &str) -> Result<(), i32> {
        let (Ok(s), Ok(p)) = (CString::new(ssid), CString::new(passwort)) else { return Err(-1) };
        // SAFETY: gültige C-Zeichenketten während des Aufrufs.
        match unsafe { esp32os_wlan_verbinden(s.as_ptr(), p.as_ptr()) } {
            0 => Ok(()),
            e => Err(e),
        }
    }

    fn trennen(&self) {
        // SAFETY: einfacher Aufruf.
        unsafe { esp32os_wlan_trennen() };
    }

    fn laden(&self) -> Vec<(String, String)> {
        crate::store::Nvs::open().ok().and_then(|n| n.lesen(NVS_SCHLUESSEL)).map(|b| netze_dekodieren(&b)).unwrap_or_default()
    }

    fn sichern(&self, netze: &[(String, String)]) -> bool {
        match crate::store::Nvs::open() {
            Ok(n) => {
                n.schreiben(NVS_SCHLUESSEL, &netze_kodieren(netze));
                log::info!("WLAN: {} bekannte Netze gespeichert", netze.len());
                true
            }
            Err(e) => {
                log::error!("WLAN: NVS: {e}");
                false
            }
        }
    }
}

/// Bildschirm dunkel (Schoner/Ruhezustand): WLAN spart Strom per iTWT (W5).
/// Setzt die Anzeigeschleife; der WLAN-Faden folgt (RPCs können warten).
pub static SPAREN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Interner RAM, den die Bildpuffer für das WLAN frei lassen (Bytes).
pub const INTERN_RESERVE: usize = 64 * 1024;

/// Freier interner RAM (für W1/W7).
fn intern_frei() -> usize {
    // SAFETY: reine Abfrage.
    unsafe { esp_idf_svc::sys::heap_caps_get_free_size(esp_idf_svc::sys::MALLOC_CAP_INTERNAL) }
}

/// WLAN im Hintergrund starten, Treiber anmelden, danach alle 0,5 s
/// `kern::wlan::wache`. Mit Build-Variablen aus `wlan.local`
/// (`bauen_flashen.ps1 -Wlan`) ist das Testnetz bekannt (Anzeige
/// anonymisiert, siehe `kern::wlan::test_netz_setzen`); ist noch kein Netz
/// gemerkt, wird einmal damit verbunden (ohne es zu merken).
/// Protokolliert nur Zustand, IP, Signal, Kanal und PHY – nie SSID oder
/// Passwort.
pub fn im_hintergrund_starten() {
    static GESTARTET: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    // Startwächter: Ein hängender Start (z. B. kein interner RAM) soll im Log
    // auffallen statt still zu bleiben.
    let _ = std::thread::Builder::new().name("wlan_w".into()).stack_size(3072).spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(15));
        if !GESTARTET.load(core::sync::atomic::Ordering::Relaxed) {
            log::error!("WLAN: Start nach 15 s nicht fertig (interner RAM frei {} KB) – hängt", intern_frei() / 1024);
        }
    });
    let _ = std::thread::Builder::new().name("wlan".into()).stack_size(8192).spawn(|| {
        let frei_vorher = intern_frei();
        let t0 = crate::hw::now_us();
        // SAFETY: einmaliger Start.
        let r = unsafe { esp32os_wlan_start() };
        GESTARTET.store(true, core::sync::atomic::Ordering::Relaxed);
        if r != 0 {
            log::error!("WLAN: Start fehlgeschlagen, Code 0x{r:x} – kein WLAN");
            return;
        }
        log::info!(
            "WLAN: Transport zum C6 bereit, WLAN gestartet in {} ms, interner RAM frei {} → {} KB",
            (crate::hw::now_us() - t0) / 1000,
            frei_vorher / 1024,
            intern_frei() / 1024
        );
        // Versionen nebenher (alte C6-Firmware antwortet nicht → 5 s warten)
        let _ = std::thread::Builder::new().name("wlan_ver".into()).stack_size(4096).spawn(|| {
            let v = versionen();
            let (a, b, c) = v.host;
            match v.cp {
                Some((x, y, z)) => log::info!(
                    "WLAN: Versionen Host {a}.{b}.{c}, Koprozessor {} {x}.{y}.{z} – {}",
                    v.cp_ziel,
                    if (x, y, z) == v.host { "gleich" } else { "VERSCHIEDEN (W2)" }
                ),
                None => log::warn!(
                    "WLAN: Versionen Host {a}.{b}.{c}, Koprozessor {} kennt die Versionsabfrage nicht (alte Firmware, siehe eh_init_evt)",
                    v.cp_ziel
                ),
            }
            #[cfg(esp32os_c6_ota)]
            if v.cp != Some(v.host) {
                // ohne Versionsantwort: alte Firmware (vor 2.6, ohne activate)
                c6_aktualisieren(v.cp.is_none());
            }
        });
        kern::wlan::treiber_setzen(Box::new(C6));
        // Updates from GitHub, only with a repository (update.rs)
        if crate::update::REPO.is_some() {
            kern::aktualisierung::quelle_setzen(Box::new(crate::update::GitHub));
        }
        // Q2: HTTPS-Abrufe der Programme (Wetter-App)
        kern::abruf::abrufer_setzen(Box::new(crate::update::Https));
        // Entwicklung (W8): Durchsatz-Messung mit werkzeuge/durchsatz.py,
        // Testdateien mit werkzeuge/datei_senden.py
        if option_env!("ESP32OS_DURCHSATZ").is_some() || option_env!("ESP32OS_DATEI_EMPFANG").is_some() {
            crate::durchsatz::starten();
        }
        // Entwicklung (Autopilot-Läufe wiederholbar): gemerkte Netze löschen
        if option_env!("ESP32OS_WLAN_VERGESSEN").is_some() {
            for s in kern::wlan::bekannte() {
                kern::wlan::vergessen(&s);
            }
            log::warn!("WLAN: gemerkte Netze gelöscht (ESP32OS_WLAN_VERGESSEN)");
        }
        log::info!("WLAN: {} bekannte Netze", kern::wlan::bekannte().len());
        let mut t_verbinden = None;
        if let (Some(s), Some(p)) = (option_env!("ESP32OS_WLAN_SSID"), option_env!("ESP32OS_WLAN_PASSWORT")) {
            kern::wlan::test_netz_setzen(s, p);
            if kern::wlan::bekannte().is_empty() {
                log::info!("WLAN: verbinde mit dem Testnetz aus wlan.local (nicht gemerkt)");
                t_verbinden = Some(crate::hw::now_us());
                if let Err(e) = kern::wlan::verbinden(s, p, false) {
                    log::error!("WLAN: Verbinden abgelehnt, Code 0x{e:x}");
                }
            }
        }
        let start = crate::hw::now_us();
        let mut ntp_laeuft = false;
        // iTWT: zuletzt angefordert (Bildschirm dunkel) – nur bei Verbindung
        let mut spart = false;
        // Alte C6-Firmware kennt iTWT nicht (keine Antwort): nicht bei jedem
        // Abdunkeln erneut 5 s warten
        let mut itwt_moeglich = true;
        let mut nach_wlan_gestartet = false;
        let mut seitenliste_gestartet = false;
        let mut update_getestet = false;
        loop {
            let jetzt_ms = ((crate::hw::now_us() - start) / 1000) as u64;
            for e in kern::wlan::wache(jetzt_ms) {
                match e {
                    kern::wlan::Ereignis::Verbunden { ip, rssi, kanal, he } => {
                        // Entwicklung (W9): Programm starten, sobald das Netz steht
                        if let Some(prog) = option_env!("ESP32OS_START_NACH_WLAN").filter(|_| !nach_wlan_gestartet) {
                            nach_wlan_gestartet = true;
                            let (name, param) = prog.split_once(' ').unwrap_or((prog, ""));
                            let r = kern::datei::starten(&format!("/sys/{name}"), param);
                            log::info!("WLAN: {prog} gestartet ({r})");
                        }
                        // Entwicklung (BR1/BR2): Seitenliste im Browser abarbeiten
                        if option_env!("ESP32OS_SEITENLISTE").is_some() && !seitenliste_gestartet {
                            seitenliste_gestartet = true;
                            crate::seitenliste::starten();
                        }
                        // Entwicklung (U1): Selbsttest HTTPS/Prüfsumme/Abbild
                        if option_env!("ESP32OS_UPDATE_TEST").is_some() && !update_getestet {
                            update_getestet = true;
                            if !crate::update::psram_faden(c"update_test", 16 * 1024, crate::update::selbsttest) {
                                log::error!("UPDATE-TEST: Faden nicht gestartet");
                            }
                        }
                        let dauer = t_verbinden.take().map(|t| format!(" in {} ms", (crate::hw::now_us() - t) / 1000)).unwrap_or_default();
                        let mut roh = ZustandRoh::default();
                        // SAFETY: Ausgabe in lokale Struktur (Wert zwischengespeichert).
                        unsafe { esp32os_wlan_zustand(&mut roh) };
                        let quelle = if roh.phy & 0x80 != 0 { "ausgehandelt" } else { "laut Zugangspunkt, C6 kann 802.11ax" };
                        log::info!(
                            "WLAN: verbunden{dauer}, IP {}, Signal {rssi} dBm, Kanal {kanal}, PHY {} ({quelle})",
                            ip_text(ip),
                            if he { "802.11ax (HE)" } else { "802.11n oder älter" }
                        );
                    }
                    kern::wlan::Ereignis::Getrennt { grund } => log::warn!("WLAN: getrennt, Grund {grund}"),
                    kern::wlan::Ereignis::Versuch { nr } => {
                        t_verbinden = Some(crate::hw::now_us());
                        log::info!("WLAN: automatischer Versuch mit bekanntem Netz Nr. {nr}");
                    }
                    kern::wlan::Ereignis::KeinBekanntesNetz => log::info!("WLAN: kein bekanntes Netz in Reichweite"),
                }
            }
            // Stromsparen (W5): bei dunklem Bildschirm iTWT vereinbaren
            let soll = SPAREN.load(core::sync::atomic::Ordering::Relaxed) && kern::wlan::zustand().verbunden && itwt_moeglich;
            if soll != spart {
                // SAFETY: einfacher Aufruf (Ergebnis als Ereignis im C-Teil).
                let r = unsafe { esp32os_wlan_sparen(c_int::from(soll)) };
                if r == 0 {
                    log::info!("WLAN: iTWT {} angefordert", if soll { "Einrichtung" } else { "Abbau" });
                } else if r == -1 {
                    // ESP_FAIL: keine Antwort vom C6 (RPC unbekannt)
                    log::warn!("WLAN: iTWT – der C6 antwortet nicht (Firmware zu alt, siehe W2); bis zum Neustart kein weiterer Versuch");
                    itwt_moeglich = false;
                } else {
                    log::warn!("WLAN: iTWT {} abgelehnt, Code 0x{r:x}", if soll { "Einrichtung" } else { "Abbau" });
                }
                spart = soll && itwt_moeglich;
            }
            if !kern::wlan::zustand().verbunden {
                spart = false;
            }
            // Internetzeit (W3b): mit WLAN starten, beim Ausschalten anhalten
            let an = kern::zeit::ntp_an();
            if an && !ntp_laeuft && kern::wlan::zustand().verbunden {
                // SAFETY: einfacher Aufruf (lwIP läuft, IP vorhanden).
                let r = unsafe { esp32os_ntp_start() };
                ntp_laeuft = r == 0;
                if ntp_laeuft {
                    log::info!("NTP: Abgleich gestartet (pool.ntp.org)");
                } else {
                    log::error!("NTP: Start fehlgeschlagen, Code 0x{r:x}");
                }
            } else if !an && ntp_laeuft {
                // SAFETY: einfacher Aufruf.
                unsafe { esp32os_ntp_stopp() };
                ntp_laeuft = false;
                log::info!("NTP: aus (Einstellungen)");
            }
            // SAFETY: einfacher Aufruf.
            if unsafe { esp32os_ntp_gestellt() } != 0 {
                kern::zeit::ntp_gemeldet();
                let l = kern::zeit::lokal(kern::zeit::unix());
                let (y, m, d) = kern::zeit::datum_aus_tagen(l.div_euclid(86_400));
                let sek = l.rem_euclid(86_400);
                log::info!(
                    "NTP: Uhr gestellt – {d:02}.{m:02}.{y} {:02}:{:02}:{:02} Ortszeit",
                    sek / 3600,
                    sek / 60 % 60,
                    sek % 60
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}
