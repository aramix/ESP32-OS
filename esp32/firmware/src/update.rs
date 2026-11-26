//! Updates über WLAN (U1): GitHub-Release als `kern::aktualisierung::Quelle`.
//! Abfrage `https://api.github.com/repos/<REPO>/releases/latest`, Dateien
//! `esp32os.bin` und `esp32os.bin.sha256` des Releases; HTTPS immer mit
//! Zertifikatsprüfung (`komponenten/esp32os/update.c`).

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;

/// Release repository, per board ([`crate::board::UPDATE_REPO`]); override at
/// build time with `ESP32OS_UPDATE_REPO=owner/name`. Without one there are no
/// updates over Wi-Fi.
pub const REPO: Option<&str> = match option_env!("ESP32OS_UPDATE_REPO") {
    Some(r) => Some(r),
    None => crate::board::UPDATE_REPO,
};

unsafe extern "C" {
    fn esp32os_https_text(url: *const c_char, json: c_int, puffer: *mut c_char, max: c_int, status: *mut c_int) -> c_int;
    fn esp32os_ota_von_url(
        url: *const c_char,
        label: *const c_char,
        sha: *const u8,
        fortschritt: Option<unsafe extern "C" fn(c_int, *mut c_void)>,
        kontext: *mut c_void,
    ) -> c_int;
    fn esp32os_boot_platz(aus: *mut c_char, n: c_int) -> c_int;
}

/// Startplatz für den nächsten Neustart (sicher auch aus Fäden mit
/// PSRAM-Stapel, siehe update.c).
pub fn boot_platz() -> String {
    let mut b = [0 as c_char; 20];
    // SAFETY: Puffer und Länge passen.
    unsafe { esp32os_boot_platz(b.as_mut_ptr(), b.len() as c_int) };
    let t: Vec<u8> = b.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&t).into_owned()
}

/// Lesbarer Text zu einem negativen Fehler aus `esp32os_https_text`
/// (negierter esp_err_t von esp_http_client_open bzw. -1/-2 aus update.c).
fn verbindungsfehler(n: c_int) -> &'static str {
    use kern::sprache::t;
    match n {
        // ESP_ERR_HTTP_CONNECT: Namensauflösung, TCP oder TLS gescheitert
        -0x7002 => t(
            "Server not reachable - does this Wi-Fi have internet access (login page)?",
            "Server nicht erreichbar - hat dieses WLAN Internet (Anmeldeseite)?",
        ),
        // ESP_ERR_HTTP_CONNECTING / ESP_ERR_HTTP_EAGAIN
        -0x7006 | -0x7007 => t("Server not answering (timeout)", "Server antwortet nicht (Zeitüberschreitung)"),
        -0x7001 | -2 => t("Too many redirects", "Zu viele Weiterleitungen"),
        _ => t("Connection failed", "Verbindung fehlgeschlagen"),
    }
}

/// Text über HTTPS holen: (HTTP-Status, Inhalt).
pub fn https_text(url: &str, json: bool) -> Result<(i32, String), String> {
    let u = CString::new(url).map_err(|_| "URL".to_string())?;
    // Über 16 KB: im PSRAM (Release-JSON mit Dateiliste wird gross)
    let mut puffer = vec![0u8; 64 * 1024];
    let mut status = 0;
    // SAFETY: Puffer und Länge passen, Zeichenkette gültig.
    let n = unsafe { esp32os_https_text(u.as_ptr(), c_int::from(json), puffer.as_mut_ptr().cast(), puffer.len() as c_int, &mut status) };
    if n < 0 {
        return Err(format!("{} ({n})", verbindungsfehler(n)));
    }
    puffer.truncate(n as usize);
    Ok((status, String::from_utf8_lossy(&puffer).into_owned()))
}

unsafe extern "C" fn fortschritt_c(p: c_int, kontext: *mut c_void) {
    // SAFETY: Kontext ist der &mut dyn FnMut aus `installieren` (lebt während des Aufrufs).
    let f = unsafe { &mut *(kontext as *mut &mut dyn FnMut(u32)) };
    f(p.clamp(0, 100) as u32);
}

pub struct GitHub;

/// Q2: HTTPS-Abruf für Programme (Wetter-App), gleiche Zertifikatsprüfung
/// wie das Update; ohne WLAN-Verbindung sofort ein Fehler in Worten.
pub struct Https;

impl kern::abruf::Abrufer for Https {
    fn text(&self, url: &str) -> Result<(i32, String), String> {
        if !kern::wlan::zustand().verbunden {
            return Err(kern::sprache::t("No Wi-Fi connection", "Keine WLAN-Verbindung").into());
        }
        https_text(url, true)
    }
}

impl kern::aktualisierung::Quelle for GitHub {
    fn neueste(&self) -> Result<kern::aktualisierung::Freigabe, String> {
        if !kern::wlan::zustand().verbunden {
            return Err(kern::sprache::t("No Wi-Fi connection", "Keine WLAN-Verbindung").into());
        }
        let repo = REPO.ok_or("ESP32OS_UPDATE_REPO")?;
        let url = format!("https://api.github.com/repos/{repo}/releases/latest");
        let (status, json) = https_text(&url, true)?;
        log::info!("UPDATE: Abfrage {url}: HTTP {status}, {} Byte", json.len());
        match status {
            200 => kern::aktualisierung::freigabe_aus_json(&json),
            404 => Err(kern::sprache::t("No release published yet", "Noch kein Release veröffentlicht").into()),
            s => Err(format!("HTTP {s}")),
        }
    }

    fn installieren(&self, f: &kern::aktualisierung::Freigabe, fortschritt: &mut dyn FnMut(u32)) -> Result<(), String> {
        let (status, text) = https_text(&f.pruef_url, false)?;
        if status != 200 {
            return Err(format!("Prüfsumme: HTTP {status}"));
        }
        let sha = kern::aktualisierung::sha256_aus_text(&text).ok_or("Prüfsummendatei unlesbar")?;
        log::info!("UPDATE: {} → {} (laufend {})", f.version, crate::slots::update_platz().to_string_lossy(), crate::slots::laufend());
        installieren_mit_sha(f, &sha, fortschritt)?;
        log::info!("UPDATE: {} geschrieben und geprüft, Neustart in den neuen Platz", f.version);
        Ok(())
    }
}

/// Faden mit Stapel im PSRAM starten (TLS braucht viel Stapel, der interne
/// RAM ist neben Anzeige und WLAN knapp – siehe W7).
pub fn psram_faden(name: &'static core::ffi::CStr, stapel: usize, f: impl FnOnce() + Send + 'static) -> bool {
    use esp_idf_svc::sys;
    // SAFETY: Konfiguration nur für das folgende pthread_create dieses Fadens.
    unsafe {
        let mut cfg = sys::esp_pthread_get_default_config();
        cfg.stack_size = stapel;
        cfg.stack_alloc_caps = sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT;
        cfg.thread_name = name.as_ptr();
        sys::esp_pthread_set_cfg(&cfg);
    }
    let ok = std::thread::Builder::new().stack_size(stapel).spawn(f).is_ok();
    // SAFETY: zurück auf die Vorgabe.
    unsafe {
        let d = sys::esp_pthread_get_default_config();
        sys::esp_pthread_set_cfg(&d);
    }
    ok
}

/// SHA-256 über mbedTLS (Selbsttest).
fn sha256(d: &[u8]) -> [u8; 32] {
    let mut aus = [0u8; 32];
    // SAFETY: Eingabe und Ausgabe passend gross.
    unsafe { esp_idf_svc::sys::mbedtls_sha256(d.as_ptr(), d.len(), aus.as_mut_ptr(), 0) };
    aus
}

/// Entwicklungs-Selbsttest (`ESP32OS_UPDATE_TEST=1`, nach der ersten
/// Verbindung): belegt HTTPS mit Zertifikatsprüfung zu GitHub, die Ablehnung
/// eines ungültigen Zertifikats und dass Prüfsumme und Abbildprüfung fremde
/// Daten nicht installieren (der Startplatz bleibt unverändert).
pub fn selbsttest() {
    log::info!("UPDATE-TEST: laufende Version {}, Platz {}, Ziel {}", desktop::shell::VERSION, crate::slots::laufend(), crate::slots::update_platz().to_string_lossy());
    // 1) Release-Abfrage über HTTPS (Zertifikat geprüft)
    match kern::aktualisierung::pruefen(desktop::shell::VERSION) {
        Ok(Some(f)) => log::info!("UPDATE-TEST: neuere Version {} gefunden", f.version),
        Ok(None) => log::info!("UPDATE-TEST: Version aktuell"),
        Err(e) => log::info!("UPDATE-TEST: Abfrage: {e}"),
    }
    // 2) Ungültiges Zertifikat muss scheitern
    match https_text("https://expired.badssl.com/", false) {
        Ok((s, _)) => log::error!("UPDATE-TEST: abgelaufenes Zertifikat ANGENOMMEN (HTTP {s}) – Fehler!"),
        Err(e) => log::info!("UPDATE-TEST: abgelaufenes Zertifikat abgelehnt wie verlangt ({e})"),
    }
    match https_text("https://self-signed.badssl.com/", false) {
        Ok((s, _)) => log::error!("UPDATE-TEST: selbst signiertes Zertifikat ANGENOMMEN (HTTP {s}) – Fehler!"),
        Err(e) => log::info!("UPDATE-TEST: selbst signiertes Zertifikat abgelehnt wie verlangt ({e})"),
    }
    // 3) Fremde Datei als „Abbild“: falsche Prüfsumme, dann richtige
    //    Prüfsumme, aber kein ESP32-Abbild → beides nicht installiert
    let url = "https://raw.githubusercontent.com/espressif/esp-idf/v5.5/README.md";
    let laufend_vorher = crate::slots::laufend();
    match https_text(url, false) {
        Ok((200, text)) => {
            let echt = sha256(text.as_bytes());
            let mut falsch = echt;
            falsch[0] ^= 0xFF;
            let mut ohne = |_p: u32| {};
            for (name, sha) in [("fremde Datei, falsche Prüfsumme", falsch), ("fremde Datei, richtige Prüfsumme", echt)] {
                let f = kern::aktualisierung::Freigabe { version: "test".into(), bild_url: url.into(), pruef_url: String::new() };
                let r = installieren_mit_sha(&f, &sha, &mut ohne);
                log::info!("UPDATE-TEST: {name}: {}", match r {
                    Ok(()) => "INSTALLIERT – Fehler!".to_string(),
                    Err(e) => format!("abgelehnt ({e})"),
                });
            }
            let name = boot_platz();
            log::info!("UPDATE-TEST: Startplatz danach {name} (läuft {laufend_vorher}) – {}", if name == laufend_vorher { "unverändert" } else { "GEÄNDERT – Fehler!" });
        }
        r => log::warn!("UPDATE-TEST: Testdatei nicht geladen: {:?}", r.map(|(s, _)| s)),
    }
}

/// OTA mit bekannter Prüfsumme (Kern von `installieren`).
fn installieren_mit_sha(f: &kern::aktualisierung::Freigabe, sha: &[u8; 32], fortschritt: &mut dyn FnMut(u32)) -> Result<(), String> {
    let ziel = crate::slots::update_platz();
    let url = CString::new(f.bild_url.as_str()).map_err(|_| "URL".to_string())?;
    let mut f_ref: &mut dyn FnMut(u32) = fortschritt;
    // SAFETY: Zeiger leben während des Aufrufs; Rückruf nur synchron.
    let r = unsafe { esp32os_ota_von_url(url.as_ptr(), ziel.as_ptr(), sha.as_ptr(), Some(fortschritt_c), (&mut f_ref as *mut &mut dyn FnMut(u32)).cast()) };
    match r {
        0 => Ok(()),
        -4 => Err(kern::sprache::t("Checksum mismatch - not installed", "Prüfsumme falsch - nicht installiert").into()),
        -3 => Err(kern::sprache::t("Download incomplete", "Download unvollständig").into()),
        -2 => Err(kern::sprache::t("Connection failed", "Verbindung fehlgeschlagen").into()),
        -1 => Err(kern::sprache::t("No update slot", "Kein Update-Platz").into()),
        e => Err(format!("{} (0x{e:x})", kern::sprache::t("Image rejected", "Abbild abgelehnt"))),
    }
}
