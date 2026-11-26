# ESP32-OS

A KolibriOS-compatible OS running natively on an ESP32-P4 touch console.

![ESP32-OS desktop on the board](docs/images/desktop.png)

| | |
|---|---|
| ![Web browser](docs/images/browser.png) | ![PDF viewer](docs/images/pdf-viewer.png) |
| ![Notes with on-screen keyboard](docs/images/notes-keyboard.png) | ![Würfelwelt](docs/images/wuerfelwelt.png) |
| ![Weather](docs/images/weather.png) | |

ESP32-OS re-implements the KolibriOS system functions in Rust instead of emulating x86. Original KolibriOS C programs are compiled for the RISC-V CPU and run unchanged. On top of that run a touch desktop, system programs, the NetSurf web browser and four games.

## Facts

Measured on the board (Waveshare ESP32-P4-WIFI6-Touch-LCD-5, chip revision v1.x, 360 MHz):

| | Value | Evidence |
|---|---|---|
| OS image | 5.6 MiB (5,825,632 bytes), 93.6 % of its 5.9 MiB flash slot | [release_paket_v1.0.3.txt](docs/nachweise/v103/release_paket_v1.0.3.txt) |
| Power-on to desktop | 3.5 s | [o4_1_apps_log.txt](docs/nachweise/o4/o4_1_apps_log.txt) |
| Browser scrolling | 32 frames/s (Wikipedia article) | [o1_scroll_log.txt](docs/nachweise/o1/o1_scroll_log.txt) |
| PDF page (A4 with photo, text, shapes) | 0.24 s | [z1_board_log.txt](docs/nachweise/z1/z1_board_log.txt) |
| Games | Pudding-Panik 55, Doom 55, Würfelwelt 27.6, Quake 27.5 frames/s | [o4/](docs/nachweise/o4/) |
| Wi-Fi throughput (TCP) | 11.6 Mbit/s to the board, 9.8 Mbit/s from the board | [w8_durchsatz.txt](docs/nachweise/w8_durchsatz.txt) |
| Endurance | 40 min stability round and 2 × 75 min endurance runs (all programs, menu, browser), 0 crashes, 0 resets | [sb_auswertung.txt](docs/nachweise/sb/sb_auswertung.txt) |

## Supported boards

| Board | Panel | Flash | Install |
|---|---|---|---|
| Waveshare ESP32-P4-WIFI6-Touch-LCD-5 (default) | 5" 720×1280 | 32 MB | releases or build from source |
| Guition JC4880P443C_I_W | 4.3" 480×800 | 16 MB | build from source ([Building](#building)); no release carries its firmware yet |

The screenshots and measurements above are from the Waveshare board.

## Quick start

This section is for the **Waveshare** board. Do not flash these release files on the Guition.

Install the latest release (1.0.3, chip revision v1.x). The web flasher still installs 1.0.1; update afterwards in *Settings → System → Check for updates*.

**Web flasher** (Chrome or Edge on a desktop computer): open the [ESP32-OS web flasher](https://theunkn0wnc0de.github.io/ESP32-OS/), connect the board's USB TO UART port, click *Connect*, choose the board's serial port and click *Install*. Without *Erase device*, settings and saved Wi-Fi networks are kept ([o8_webflasher.txt](docs/nachweise/o8/o8_webflasher.txt)).

**espflash:**

1. Download the files of the [latest release](https://github.com/TheUnkn0wnC0DE/ESP32-OS/releases/latest) and check them against `SHA256SUMS`.
2. Connect the board's USB TO UART port.
3. Flash with [espflash](https://github.com/esp-rs/espflash):
   ```powershell
   espflash write-bin 0x2000 bootloader.bin
   espflash write-bin 0x8000 partition-table.bin
   espflash write-bin 0x10000 otadata-leer.bin
   espflash write-bin 0x20000 esp32os.bin
   espflash write-bin 0xC00000 doomapp.bin
   espflash write-bin 0xE00000 quakeapp.bin
   ```
4. Optional (both ways): copy the shareware files `DOOM1.WAD` to `ESP32-OS/doom/` and `pak0.pak` to `ESP32-OS/quake/` on the microSD card.

Boards with chip revision v3.x and changes to the code: build from source, see [Building](#building).

## Features

| Area | What works |
|---|---|
| Kernel | KolibriOS system functions in Rust, one heap per program, crash isolation with error dialog, compositor with dirty rectangles, two firmware slots with automatic rollback |
| KolibriOS programs | Original C programs, unchanged: PasswordGen, TinyHashView and the clayer samples; native box_lib, libimg, proc_lib, libini, network |
| Desktop | Icons, taskbar with Wi-Fi signal and clock, start menu, tap, long press, swipe and pinch, on-screen keyboard (Swiss German, US), English and German interface, first-run wizard |
| Programs | File manager (microSD and internal flash), image viewer (JPEG in hardware, PNG, GIF, BMP; phone photos up to 48 megapixels), PDF viewer, settings, notes, calculator, clock with alarm and timer, calendar, weather (Open-Meteo, 5-day forecast, no API key), music player (MP3, WAV), task manager, screenshot tool, console with the KolibriOS shell (pipes, redirection) |
| Wi-Fi 6 | Via the ESP32-C6: scan and connect by touch, remembered networks, time via NTP |
| Browser | NetSurf 3.11, HTTPS with certificate check, tabs, bookmarks, history, downloads, zoom, forms, JavaScript (QuickJS-ng: DOM, events, timers, fetch, cookies, localStorage) |
| Games | Pudding-Panik, Würfelwelt, Doom, Quake, started from the desktop |
| Updates | Firmware update over Wi-Fi from GitHub releases with SHA-256 check and rollback; tested on the board from 0.9 to 1.0 ([u1_update.txt](docs/nachweise/u1/u1_update.txt)) |

## Limitations

- Wi-Fi passwords are stored unencrypted in flash (no flash encryption, no eFuses).
- The browser has 10 MB of memory. Large web applications do not load; some web APIs are missing.
- The PDF viewer shows text in the system font, opens files up to 4 MB and does not decode CCITT, JBIG2 or JPEG 2000 images.
- The image viewer scales large JPEG and PNG photos down while decoding (JPEG by 1/2, 1/4 or 1/8). A 12-megapixel phone photo opens in 5.9 s, a 48-megapixel image in 11.4 s; 39 to 45 % of that is reading the microSD card ([p2g_messung.txt](docs/nachweise/p2_gross/p2g_messung.txt)). Progressive JPEGs need more memory: 12 megapixels open, 48 megapixels are refused. GIF and BMP open up to about 0.85 megapixels.
- All programs share one address space. A faulty program is stopped and reported, but can still disturb the system.
- No battery gauge and no camera support yet (planned for 1.1).
- Release images combine GPL-2.0 code (NetSurf, KolibriOS) with Apache-2.0 code (ESP-IDF). Whether they may be distributed in this form is being clarified; the source code is not affected.
- The Windows build script builds for the Waveshare board only; the Guition is built with the macOS/Linux script (tried on macOS only).
- Guition: no updates over Wi-Fi and a single firmware slot, so there is no previous version to roll back to; about 5.9 MB of internal storage. Sound, the microSD slot and Quake have not been tested on it.

Full list with reasons: `docs/development/PLAN.md`, section "Abweichungen" (German).

## Hardware

### Waveshare ESP32-P4-WIFI6-Touch-LCD-5

Waveshare ESP32-P4-WIFI6-Touch-LCD-5 ([documentation](https://docs.waveshare.com/ESP32-P4-WIFI6-Touch-LCD-5)):

- ESP32-P4, dual-core RISC-V (360 MHz on chip revision v1.x, 400 MHz on v3.x), 32 MB PSRAM, 32 MB flash, hardware JPEG decoder, 2D accelerator (PPA)
- ESP32-C6 as Wi-Fi 6 co-processor (SDIO, esp_hosted)
- 5" IPS panel 720×1280 (MIPI-DSI), capacitive touch (GT911)
- ES8311 audio codec with speaker, microSD slot, LiPo connector

The desktop is drawn at 640×360 and scaled ×2 to the panel in landscape. A 3D-printable case is in [`esp32/gehaeuse/`](esp32/gehaeuse/).

### Guition JC4880P443C_I_W

- ESP32-P4 (tested with chip revision v1.0), 32 MB PSRAM, 16 MB flash
- ESP32-C6 as Wi-Fi co-processor (SDIO, esp_hosted), 2.4 GHz
- 4.3" IPS panel 480×800 (MIPI-DSI, ST7701), capacitive touch (GT911)
- ES8311 audio codec, microSD slot
- Log and flashing over the P4's USB port (USB Serial/JTAG)

The same 640×360 desktop is scaled ×1.25 to 800×450, with a thin black bar above and below.

## Building

### macOS or Linux (both boards)

Requirements: Git, Python 3 with Pillow, perl, [rustup](https://rustup.rs), LLVM (`brew install llvm` on macOS) and `cargo install ldproxy espflash --locked`. ESP-IDF 5.5 is downloaded on the first build. Tried on macOS only.

```sh
cd esp32/firmware
./build_flash.sh --board guition                                # build all images
./build_flash.sh --board guition --port /dev/cu.usbmodem101     # build and flash
./build_flash.sh --board guition --release 1.0.4                # Guition release files (publishes nothing)
```

Use `--board waveshare` for the Waveshare board (chip revision v1.x). Flashing replaces the firmware and partition table on the board. Doom and Quake also need their game data; without a microSD card, there is no ready-made tool to put it on the internal storage.

### Windows (Waveshare)

Requirements (Windows, PowerShell): Git, Python 3, [rustup](https://rustup.rs), LLVM (`winget install LLVM.LLVM`) and `cargo install ldproxy espflash --locked`. The Rust toolchain is pinned in `esp32/firmware/rust-toolchain.toml`. ESP-IDF 5.5 is downloaded on the first build (1 to 2 GB).

1. Fetch the browser sources (NetSurf, QuickJS-ng, zlib; SHA-256 checked, stored in `C:\ns`):
   ```powershell
   python esp32\netsurf\werkzeuge\quellen_holen.py
   ```
2. Build all images:
   ```powershell
   powershell -File esp32\firmware\bauen_flashen.ps1 -NurBauen
   ```
   The default profile is for chip revision v1.x. For v3.x set `esp_idf_sdkconfig_defaults` in `esp32/firmware/Cargo.toml` to `sdkconfig.defaults.rev3_x` and run `cargo clean` once. The script maps the repository to drive `O:` for short paths.
3. Flash (use your COM port):
   ```powershell
   powershell -File esp32\firmware\bauen_flashen.ps1 -Port COM3
   ```
4. Optional game data for local use: `python esp32\doom\wad\holen.py` and `python esp32\quake\pak\holen.py`.

PC tests (Git Bash): `bash esp32/werkzeuge/alle_tests.sh`. A release package is built with `esp32\firmware\release_paket.ps1 -Version 1.0`.

## Repository layout

| Path | Content |
|---|---|
| [`esp32/`](esp32/) | ESP32-OS source: kernel, KolibriOS layer, desktop, browser, games, firmware |
| [`spatz/`](spatz/) | software 3D engine used by the games |
| [`docs/`](docs/) | images, release notes, evidence, development documents |
| [`LICENSES/`](LICENSES/) | license texts of all third-party components |

## License

ESP32-OS is licensed under the GNU General Public License v2.0 ([`LICENSE`](LICENSE)). The `spatz` crate is MIT OR Apache-2.0; built into the firmware it is part of the GPL-2.0 work. Third-party components and their licenses are listed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md). Doom and Quake game data are not included.

## Press

- [XDA Developers](https://www.xda-developers.com/someone-used-claude-code-to-make-a-kolibrios-compatible-os-that-runs-off-an-esp32-p4/), 25 November 2026

## Credits

- [KolibriOS](https://kolibrios.org): system function interface, C programs, libraries, skin, fonts and icons
- [NetSurf](https://www.netsurf-browser.org): web browser
- [QuickJS-ng](https://github.com/quickjs-ng/quickjs): JavaScript engine
- [doomgeneric](https://github.com/ozkl/doomgeneric): Doom port
- [quakegeneric](https://github.com/erysdren/quakegeneric): Quake port

ESP32-OS was developed with Claude Code.

Follow the progress on TikTok: [@The_Unknown_Coder](https://www.tiktok.com/@The_Unknown_Coder)
