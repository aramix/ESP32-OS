#!/usr/bin/env python3
"""Holt ESP-IDF-Komponenten, die der Komponenten-Manager unter Windows nicht
selbst einrichten kann, als gekürzte lokale Kopie (W1).

esp_hosted enthält Beispiele und Tests mit sehr tiefen Ordnern. Der
Komponenten-Manager kopiert jede Komponente in den Build-Ordner von
esp-idf-sys (`C:/eo/…/out/managed_components/…`), und dort sprengen diese
Pfade die Windows-Grenze von 260 Zeichen. Darum: Archiv der festen Version
aus der Espressif-Registry laden, SHA-256 prüfen, ohne Beispiele, Tests,
Doku und Werkzeuge nach `C:/ec/v<version>/espressif__<name>` entpacken.
`komponenten/esp32os/idf_component.yml` verweist per `override_path` darauf
(gilt projektweit, auch für die Abhängigkeit aus esp_wifi_remote).

    python esp32/firmware/werkzeuge/komponenten_holen.py
"""
import hashlib
import io
import os
import sys
import urllib.request
import zipfile
from pathlib import Path

# Windows: the short path C:/ec; elsewhere in the home directory. `ESP32OS_EC`
# must also be set when building (komponenten/esp32os/idf_component.yml).
ZIEL = Path(os.environ.get("ESP32OS_EC") or ("C:/ec" if os.name == "nt" else Path.home() / ".cache/esp32os/ec"))

# Name, Version, SHA-256 des Registry-Archivs
KOMPONENTEN = [
    (
        "esp_hosted",
        "3.1.0",
        "https://components-file.espressif.com/components/espressif/esp_hosted/3.1.0/espressif__esp_hosted-v3.1.0.zip",
        "6879c4babc9c1b8888dec9a019c497980f660395950cbb0a342f95d1cbc591f0",
    ),
]

# Ordner, die für den Host-Build auf dem ESP32-P4 nicht gebraucht werden
WEG_OBEN = ("examples/", "tests/", "docs/", "port/os/stm32/", "port/os/posix/")
# Aus tools/ braucht der Build nur die CMake-Hilfen.
BEHALTEN = ("tools/cmake/",)
# Bei geänderten Regeln oder Anpassungen wird neu entpackt (Marke =
# SHA-256 + Regelstand).
REGELN = "5"
WEG_TEIL = ("/test/", "/tests/", "/test_apps/", "/examples/", "/docs/")


# Anpassungen (W7): (Komponente, Datei, alter Text, neuer Text). Der alte
# Text muss genau so vorkommen, sonst bricht das Skript ab (neue Version →
# Anpassung prüfen).
#
# esp_hosted holt seine SDIO-Puffer mit MALLOC_CAP_DMA, auf dem ESP32-P4 also
# nur aus dem internen RAM; die Option EH_HOST_PORT_DMA_PREFER_SPIRAM fragt
# PSRAM mit MALLOC_CAP_DMA an, das der P4 so nicht meldet – sie wirkt nicht.
# Der SDMMC-Treiber des P4 kann DMA mit PSRAM-Puffern (Cache-Abgleich,
# Ausrichtung auf die Cache-Zeile, sdmmc_host_check_buffer_alignment). Darum
# hier: mit der Option PSRAM auf die Cache-Zeile ausgerichtet, Länge
# aufgerundet; sonst wie bisher intern.
_DMA_ALT_1 = """void *eh_host_port_dma_alloc(size_t n)
{
    return heap_caps_malloc(n, MALLOC_CAP_DMA);
}"""
_DMA_NEU_1 = """#if CONFIG_EH_HOST_PORT_DMA_PREFER_SPIRAM
/* ESP32-OS (W7): PSRAM, ausgerichtet auf die L2-Cache-Zeile (PSRAM hängt am
 * L2-Cache), Länge aufgerundet – DMA des SDMMC-Hosts über den Cache, siehe
 * sdmmc_host_check_buffer_alignment. */
#ifdef CONFIG_CACHE_L2_CACHE_LINE_SIZE
#define EH_PSRAM_ZEILE CONFIG_CACHE_L2_CACHE_LINE_SIZE
#else
#define EH_PSRAM_ZEILE 128
#endif
static void *eh_psram_dma(size_t n, size_t alignment)
{
    size_t a = EH_PSRAM_ZEILE;
    if (alignment > a) a = alignment;
    size_t len = (n + a - 1) & ~(a - 1);
    return heap_caps_aligned_alloc(a, len, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
}
#endif

void *eh_host_port_dma_alloc(size_t n)
{
#if CONFIG_EH_HOST_PORT_DMA_PREFER_SPIRAM
    void *p = eh_psram_dma(n, 4);
    if (p) return p;
#endif
    return heap_caps_malloc(n, MALLOC_CAP_DMA);
}"""
_DMA_ALT_2 = """    ptr = heap_caps_aligned_alloc(alignment, n,
            MALLOC_CAP_SPIRAM | MALLOC_CAP_DMA | MALLOC_CAP_8BIT);"""
_DMA_NEU_2 = """    ptr = eh_psram_dma(n, alignment);"""
# W8: Alte C6-Firmware (0.0.x) meldet ihren SDIO-Modus nicht (kein
# SDIO_MODE-TLV), sendet aber mehrere Pakete am Stück (am Board: 3072 Byte).
# esp_hosted legt die Empfangspuffer dann fest auf ein Paket (1536 Byte) an
# und verweigert danach jede größere Übertragung – der Empfang bleibt
# stehen. Ohne TLV darum dynamische Puffer (wachsen bei Bedarf, im PSRAM),
# wie esp_hosted es schon für unbekannte Warteschlangengrößen macht.
_RX_ALT = """        if (s_slave_sdio_streaming_mode && !slave_tx_q_size &&
            !eh_host_mcu_transport_sdio_sw_aggr_negotiated()) {"""
_RX_NEU = """        /* ESP32-OS: alte CP-Firmware ohne SDIO_MODE-TLV streamt trotzdem
         * (mehrere Pakete je Lesevorgang) – Puffer dynamisch lassen. */
        if (!saw_sdio_mode_tlv && !eh_host_mcu_transport_sdio_sw_aggr_negotiated()) {
            ESP_LOGW(TAG, "CP without SDIO_MODE TLV: SDIO RX staging buffers stay dynamic");
            target_size = 0;
        }
        if (s_slave_sdio_streaming_mode && !slave_tx_q_size &&
            !eh_host_mcu_transport_sdio_sw_aggr_negotiated()) {"""
ANPASSUNGEN = [
    ("esp_hosted", "port/os/idf/src/eh_host_port_dma.c", _DMA_ALT_1, _DMA_NEU_1),
    ("esp_hosted", "port/os/idf/src/eh_host_port_dma.c", _DMA_ALT_2, _DMA_NEU_2),
    ("esp_hosted", "host/mcu/eh_host_mcu_transport/src/eh_host_mcu_transport_init_event.c", _RX_ALT, _RX_NEU),
]


def gebraucht(pfad: str) -> bool:
    if pfad.startswith(WEG_OBEN):
        return False
    if pfad.startswith("tools/") and not pfad.startswith(BEHALTEN):
        return False
    return not any(t in "/" + pfad for t in WEG_TEIL)


def holen(name: str, version: str, url: str, sha: str) -> None:
    # Ordnername = Build-Name der Komponente (ESP-IDF benennt Komponenten nach
    # ihrem Ordner; esp_wifi_remote verlangt „espressif__esp_hosted“).
    ordner = ZIEL / f"v{version}" / f"espressif__{name}"
    marke = ordner / ".esp32os_fertig"
    if marke.exists() and marke.read_text().strip() == f"{sha}:{REGELN}":
        print(f"{name} {version}: vorhanden ({ordner})")
        return
    print(f"{name} {version}: lade {url}")
    anfrage = urllib.request.Request(url, headers={"User-Agent": "esp32os-build"})
    with urllib.request.urlopen(anfrage, timeout=120) as r:
        daten = r.read()
    ist = hashlib.sha256(daten).hexdigest()
    if ist != sha:
        sys.exit(f"{name}: SHA-256 {ist} ≠ erwartet {sha} – abgebrochen")
    # Ein unvollständiger Ordner (ohne Marke) wird überschrieben, nicht gelöscht.
    n = weg = 0
    with zipfile.ZipFile(io.BytesIO(daten)) as z:
        for e in z.infolist():
            if e.is_dir():
                continue
            if not gebraucht(e.filename):
                weg += 1
                continue
            ziel = ordner / e.filename
            ziel.parent.mkdir(parents=True, exist_ok=True)
            ziel.write_bytes(z.read(e))
            n += 1
    for komp, datei, alt, neu in ANPASSUNGEN:
        if komp != name:
            continue
        p = ordner / datei
        t = p.read_text(encoding="utf-8")
        if alt not in t:
            sys.exit(f"{name}: Anpassung an {datei} passt nicht (neue Version?) – abgebrochen")
        p.write_text(t.replace(alt, neu, 1), encoding="utf-8", newline="")
        print(f"{name}: {datei} angepasst")
    marke.write_text(f"{sha}:{REGELN}")
    print(f"{name} {version}: {n} Dateien nach {ordner}, {weg} weggelassen")


def main() -> None:
    for k in KOMPONENTEN:
        holen(*k)


if __name__ == "__main__":
    main()
