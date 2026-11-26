# firmware

ESP-IDF firmware for the Waveshare ESP32-P4-WIFI6-Touch-LCD-5: display (MIPI-DSI, PPA scaling), touch, audio, Wi-Fi through the ESP32-C6 (`esp_hosted`), microSD, flash file systems, OTA slots and the hardware JPEG decoder. It hosts the ESP32-OS kernel (`../kern`) and desktop (`../desktop`); Doom and Quake are built as separate images (`doomapp`, `quakeapp`). This is its own Cargo workspace with a pinned toolchain (`rust-toolchain.toml`).

## Boards

| | Waveshare ESP32-P4-WIFI6-Touch-LCD-5 (default) | Guition JC4880P443C_I_W (`--features guition`) |
|---|---|---|
| Panel | HX8394, 720×1280; desktop ×2 | ST7701, 480×800; desktop ×1.25 with black bars |
| Console | UART (“USB TO UART”) | USB Serial/JTAG |
| Flash | 32 MB, `partitions.csv` | 16 MB, `partitions_guition.csv`: one OS slot, about 5.9 MB `daten` |
| Updates over Wi-Fi | from GitHub releases | off, because the releases are built for the Waveshare board. `ESP32OS_UPDATE_REPO` enables them for your own repository, which also needs a second OTA slot |

Board differences live in `src/board/` and `src/serial/`. The Guition build also needs `sdkconfig.defaults.guition`; a mismatch with the Cargo feature is a compile error.

Build and flash on macOS or Linux (either board; needs rustup, `cargo install ldproxy espflash --locked`, LLVM, perl and Python 3 with Pillow):

```sh
./build_flash.sh --board guition                                # build all images
./build_flash.sh --board guition --port /dev/cu.usbmodem101     # build and flash
```

Release files for the web flasher (`docs/flasher/`, which lets users pick their board):

* **Waveshare:** `release_paket.ps1` (Windows) writes `manifest.json`, the images and `SHA256SUMS`.
* **Guition:** `./build_flash.sh --board guition --release <version>` writes the same set with a `guition-` prefix to `release/guition/v<version>/`. It publishes nothing.

Upload either or both sets to the same GitHub release, then run `gh workflow run flasher.yml --ref main`. The page disables a board whose manifest is missing.

Build and flash for Waveshare on Windows (PowerShell):

```powershell
powershell -File esp32\firmware\bauen_flashen.ps1 -NurBauen              # build all images
powershell -File esp32\firmware\bauen_flashen.ps1 -Nur esp32os -Port COM3 # build and flash the OS only
powershell -File esp32\firmware\release_paket.ps1 -Version 1.0           # release package into release\
```

`-Release` refuses Wi-Fi test credentials and development switches and checks the finished images. Flash layout: `partitions.csv`. Serial tools: `seriell_log.py` (log), `bildschirmfoto.py` (framebuffer screenshots); further helpers are in `werkzeuge/`.
