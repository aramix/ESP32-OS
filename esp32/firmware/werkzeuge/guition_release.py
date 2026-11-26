#!/usr/bin/env python3
"""Release files of the Guition JC4880P443C_I_W for GitHub releases and the
web flasher (the Waveshare ones come from release_paket.ps1). Publishes
nothing. Called by `build_flash.sh --board guition --release VERSION`.

    guition_release.py VERSION IMAGE_DIR BOOTLOADER OUT_DIR

All names start with "guition-", so both boards fit into one release. The
Waveshare updater only takes a file named exactly "esp32os.bin".

Aborts, writing nothing, if an image contains game data, a value from
wlan.local, the home directory or the user name.
"""
import csv
import hashlib
import json
import subprocess
import sys
from pathlib import Path

FIRMWARE = Path(__file__).resolve().parent.parent
REPO = FIRMWARE.parent.parent
APPS = {"esp32os": "esp32os_a", "doomapp": "doomapp", "quakeapp": "quakeapp"}


def partitions():
    """Name -> (offset, size), from partitions_guition.csv."""
    rows = csv.reader((FIRMWARE / "partitions_guition.csv").read_text().splitlines())
    return {r[0].strip(): (int(r[3], 0), int(r[4], 0)) for r in rows if r and not r[0].startswith("#")}


def forbidden():
    """(what, bytes) that must not appear in a published image."""
    home = Path.home()
    found = [("home directory", str(home).encode()), ("user name", home.name.encode())]
    wlan = REPO / "wlan.local"
    if wlan.exists():
        for line in wlan.read_text(encoding="utf-8").splitlines():
            key, _, value = line.partition("=")
            if key.strip() in ("SSID", "PASSWORT") and value.strip():
                found.append((f"{key.strip()} from wlan.local", value.strip().encode()))
    return found


def main():
    version, images, bootloader, out = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[4])
    table = partitions()
    files = {"bootloader.bin": (0x2000, bootloader.read_bytes())}
    for app, partition in APPS.items():
        offset, size = table[partition]
        data = (images / f"{app}.bin").read_bytes()
        if len(data) > size:
            sys.exit(f"{app}.bin: {len(data)} bytes do not fit into {partition} ({size})")
        files[f"{app}.bin"] = (offset, data)

    for name, (_, data) in files.items():
        for what, needle in forbidden():
            if needle in data:
                # never print the value itself
                sys.exit(f"{name} contains the {what} - no package written")
    check = subprocess.run([sys.executable, "-I", str(REPO / "esp32/werkzeuge/spieldaten_pruefen.py"),
                            *[str(images / f"{app}.bin") for app in APPS]])
    if check.returncode:
        sys.exit("game data in an image - no package written")

    out.mkdir(parents=True, exist_ok=True)
    table_bin = out / "guition-partition-table.bin"
    subprocess.run(["espflash", "partition-table", str(FIRMWARE / "partitions_guition.csv"),
                    "--to-binary", "--output", str(table_bin)], check=True)
    files["partition-table.bin"] = (0x8000, table_bin.read_bytes())
    # erased otadata: the board starts in esp32os_a
    offset, size = table["otadata"]
    files["otadata-leer.bin"] = (offset, b"\xff" * size)

    sums = []
    for name, (_, data) in sorted(files.items()):
        (out / f"guition-{name}").write_bytes(data)
        sums.append(f"{hashlib.sha256(data).hexdigest()}  guition-{name}\n")
    (out / "guition-SHA256SUMS").write_text("".join(sums))
    manifest = {
        "name": "ESP32-OS (Guition JC4880P443C_I_W)",
        "version": version,
        "new_install_prompt_erase": True,
        "builds": [{"chipFamily": "ESP32-P4",
                    "parts": [{"path": f"guition-{n}", "offset": o} for n, (o, _) in sorted(files.items(), key=lambda f: f[1][0])]}],
    }
    (out / "guition-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Release files ready in {out} (nothing published)")


if __name__ == "__main__":
    main()
