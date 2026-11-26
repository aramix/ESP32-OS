// Gibt die Link- und Umgebungsdaten von ESP-IDF an Cargo weiter.
fn main() {
    embuild::espidf::sysenv::output();
    // used by the board check in src/lib.rs
    println!("cargo:rustc-check-cfg=cfg(esp_idf_esp_console_usb_serial_jtag)");
    // Entwicklungs-Build mit Testbildern (P2-Nachweis)
    println!("cargo:rustc-check-cfg=cfg(esp32os_testbilder)");
    println!("cargo:rerun-if-env-changed=ESP32OS_TESTBILDER");
    if std::env::var_os("ESP32OS_TESTBILDER").is_some() {
        println!("cargo:rustc-cfg=esp32os_testbilder");
    }
    // Entwicklungs-Build mit Testtönen für den Musikplayer (A5-Nachweis)
    println!("cargo:rustc-check-cfg=cfg(esp32os_musik_test)");
    println!("cargo:rerun-if-env-changed=ESP32OS_MUSIK_TEST");
    if std::env::var_os("ESP32OS_MUSIK_TEST").is_some() {
        println!("cargo:rustc-cfg=esp32os_musik_test");
    }
    // Entwicklungs-Build mit Heap-Tracing des Systemheaps (BR1: Leck im
    // internen RAM); braucht CONFIG_HEAP_TRACING_STANDALONE im sdkconfig
    println!("cargo:rustc-check-cfg=cfg(esp32os_heap_trace)");
    println!("cargo:rerun-if-env-changed=ESP32OS_HEAP_TRACE");
    if std::env::var_os("ESP32OS_HEAP_TRACE").is_some() {
        println!("cargo:rustc-cfg=esp32os_heap_trace");
    }
    // C2 (W2): Entwicklungs-Build, der die eigene C6-Firmware vom P4 aus
    // schreibt (Bild aus werkzeuge/c6_bauen.ps1, Bauordner C:\eo\c6)
    println!("cargo:rustc-check-cfg=cfg(esp32os_c6_ota)");
    println!("cargo:rerun-if-env-changed=ESP32OS_C6_OTA");
    if std::env::var_os("ESP32OS_C6_OTA").is_some() {
        let bild = std::env::var("ESP32OS_C6_BILD").unwrap_or_else(|_| "C:/eo/c6/esp32os_c6.bin".into());
        assert!(std::path::Path::new(&bild).exists(), "C6-Bild {bild} fehlt – zuerst werkzeuge/c6_bauen.ps1");
        println!("cargo:rerun-if-changed={bild}");
        println!("cargo:rustc-env=ESP32OS_C6_BILD={bild}");
        println!("cargo:rustc-cfg=esp32os_c6_ota");
    }
    // Autopilot aus einer Datei (lange Abläufe: Umgebungsvariablen sind unter
    // Windows auf 32767 Zeichen begrenzt; Stabilitätsrunde 66 KB)
    println!("cargo:rustc-check-cfg=cfg(esp32os_autopilot_datei)");
    println!("cargo:rerun-if-env-changed=ESP32OS_AUTOPILOT_DATEI");
    if let Ok(p) = std::env::var("ESP32OS_AUTOPILOT_DATEI") {
        assert!(std::path::Path::new(&p).exists(), "Autopilot-Datei {p} fehlt");
        println!("cargo:rerun-if-changed={p}");
        println!("cargo:rustc-env=ESP32OS_AUTOPILOT_PFAD={p}");
        println!("cargo:rustc-cfg=esp32os_autopilot_datei");
    }
    // Entwicklungs-Build mit Test-PDFs für den PDF-Betrachter (Z1-Nachweis)
    println!("cargo:rustc-check-cfg=cfg(esp32os_pdf_test)");
    println!("cargo:rerun-if-env-changed=ESP32OS_PDF_TEST");
    if std::env::var_os("ESP32OS_PDF_TEST").is_some() {
        println!("cargo:rustc-cfg=esp32os_pdf_test");
    }
}
