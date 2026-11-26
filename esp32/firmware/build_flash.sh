#!/usr/bin/env bash
# Build and flash ESP32-OS on macOS/Linux.
# (Windows, Waveshare only: bauen_flashen.ps1.)
#
#   ./build_flash.sh --board guition                             # build all three programs
#   ./build_flash.sh --board guition --port /dev/cu.usbmodem101  # build, then flash
#
#   ./build_flash.sh --board guition --release 1.0.4             # release files, no flashing
#
# --release builds without local paths, refuses development switches and
# checks the images (werkzeuge/guition_release.py); they go to
# release/guition/v<version>/. Waveshare releases: release_paket.ps1.
#
# Boards: guition (JC4880P443C_I_W), waveshare (ESP32-P4-WIFI6-Touch-LCD-5,
# chip revision v1.x). Each board builds into its own target/<board>.
#
# Needs: rustup, `cargo install ldproxy espflash --locked`, LLVM (libclang),
# Python 3 with Pillow, perl.
set -euo pipefail

port="" board="" release=""
while [ $# -gt 0 ]; do
    case "$1" in
        --board) board="${2:?--board needs guition or waveshare}"; shift 2 ;;
        --release) release="${2:?--release needs a version}"; shift 2 ;;
        --port) port="${2:?--port needs a device}"; shift 2 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

# no default: flashing the other board's image would leave it dark
case "$board" in
    guition)
        flash=16mb table=partitions_guition.csv doom_at=0x610000 quake_at=0x810000
        defaults="sdkconfig.defaults;sdkconfig.defaults.rev1_3;sdkconfig.defaults.guition"
        board_feature=guition ;;
    waveshare)
        flash=32mb table=partitions.csv doom_at=0xC00000 quake_at=0xE00000
        defaults="sdkconfig.defaults;sdkconfig.defaults.rev1_3"
        board_feature="" ;;
    *) echo "--board guition or --board waveshare is required" >&2; exit 2 ;;
esac

cd "$(dirname "$0")"
export ESP_IDF_SDKCONFIG_DEFAULTS="$defaults"
export CARGO_WORKSPACE_DIR="$PWD"
target="target/$board"
remap=()
if [ -n "$release" ]; then
    [ "$board" = guition ] || { echo "--release is for guition (Waveshare: release_paket.ps1)" >&2; exit 2; }
    [ -z "$port" ] || { echo "--release does not flash; leave out --port" >&2; exit 2; }
    built="$(sed -n 's/.*pub const VERSION: &str = "\([^"]*\)".*/\1/p' ../desktop/src/shell.rs)"
    case "$built" in
        "$release"|"$release "*) ;;
        *) echo "built-in version '$built' does not match --release $release (desktop/src/shell.rs)" >&2; exit 1 ;;
    esac
    # Wi-Fi test credentials and development switches act at compile time
    switches="$(env | cut -d= -f1 | grep -v '^ESP32OS_EC$' | grep -E '^(ESP32OS_|KERN_.*TEST$|KONSOLE_(AUTOPILOT|BILDER|RUNDGANG|START|WEICH)$)' || true)"
    [ -z "$switches" ] || { echo "release: unset first: $(echo $switches)" >&2; exit 1; }
    # own build folder, configured from scratch with path remapping for C and Rust
    target="target/$board-release"
    export ESP32OS_HIDE_HOME="$HOME" # C code: komponenten/esp32os/project_include.cmake
    remap=(--config "target.riscv32imafc-esp-espidf.rustflags=['--remap-path-prefix=$HOME=~']")
fi
ns_root="$HOME/.cache/esp32os/ns"
export ESP32OS_EC="${ESP32OS_EC:-$HOME/.cache/esp32os/ec}"
export NETSURF_QUELLEN="$ns_root/3.11"
if [ -z "${LIBCLANG_PATH:-}" ] && command -v brew >/dev/null; then
    export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
fi
# esp-idf-sys brings its own ESP-IDF; an activated one would override it.
unset IDF_PATH

python3 werkzeuge/komponenten_holen.py
python3 ../netsurf/werkzeuge/quellen_holen.py
# ESP-IDF compiles NetSurf from a file list that the netsurf crate writes.
(cd .. && NETSURF_LIST_ONLY=1 cargo build -p netsurf)

mkdir -p "$target/abbilder"
for program in esp32os:"" doomapp:doom quakeapp:quake; do
    name="${program%%:*}" feature="${program#*:}"
    features="$board_feature${board_feature:+${feature:+,}}$feature"
    CARGO_TARGET_DIR="$target" cargo build --release --bin pudding-p4 ${remap[@]+"${remap[@]}"} ${features:+--features "$features"}
    cp "$target/riscv32imafc-esp-espidf/release/pudding-p4" "$target/abbilder/$name.elf"
    espflash save-image --chip esp32p4 --flash-size "$flash" "$target/abbilder/$name.elf" "$target/abbilder/$name.bin"
done

# Bootloader built by ESP-IDF (with rollback support)
boot="$(find "$target/riscv32imafc-esp-espidf/release/build" -path '*/out/build/bootloader/bootloader.bin' -print0 | xargs -0 ls -t | head -1)"
if [ -n "$release" ]; then
    python3 werkzeuge/guition_release.py "$release" "$target/abbilder" "$boot" "../../release/guition/v$release"
fi
[ -n "$port" ] || exit 0
# Erasing otadata makes the board start in esp32os_a.
espflash flash --port "$port" --baud 921600 --flash-size "$flash" --bootloader "$boot" \
    --partition-table "$table" --target-app-partition esp32os_a --erase-parts otadata \
    --non-interactive "$target/abbilder/esp32os.elf"
espflash write-bin --port "$port" --baud 921600 --non-interactive "$doom_at" "$target/abbilder/doomapp.bin"
espflash write-bin --port "$port" --baud 921600 --non-interactive "$quake_at" "$target/abbilder/quakeapp.bin"
