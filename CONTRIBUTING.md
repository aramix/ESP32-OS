# Contributing to ESP32-OS

Thanks for your interest in ESP32-OS! Bug reports, ideas, tests on your own board and pull requests are all welcome. Issues and pull requests can be written in English or German.

## Before you start

- Read the [README](README.md), especially *Limitations*. Some missing features are known and planned.
- Search the [issues](https://github.com/TheUnkn0wnC0DE/ESP32-OS/issues) first; maybe someone already reported it.
- Questions and ideas that are not clearly a bug fit best in [Discussions](https://github.com/TheUnkn0wnC0DE/ESP32-OS/discussions).
- For larger changes (new program, kernel or memory changes, new hardware), open an issue first so we can agree on the approach before you spend time on it.
- Security problems: do **not** open a public issue, see [SECURITY.md](SECURITY.md).

## Reporting bugs

Use the *Bug report* template. The most helpful information is:

- ESP32-OS version (*Settings → System*) and how you installed it (web flasher, espflash, own build)
- Chip revision of your board (v1.x or v3.x)
- Exact steps to reproduce, what you expected and what happened
- The serial log from the USB TO UART port, if you can capture it (for example with `espflash monitor`)
- A photo or screenshot (the screenshot tool is in the start menu) if it is a display problem

## Development setup

Requirements and steps are in the README under [Building](README.md#building). On macOS or Linux use `esp32/firmware/build_flash.sh --board waveshare|guition`; the PC tests are set up for Windows. In short, on Windows (Waveshare):

```powershell
python esp32\netsurf\werkzeuge\quellen_holen.py
powershell -File esp32\firmware\bauen_flashen.ps1 -NurBauen
powershell -File esp32\firmware\bauen_flashen.ps1 -Port COM3
```

Run the PC tests (Git Bash) before you open a pull request:

```bash
bash esp32/werkzeuge/alle_tests.sh
```

## Pull requests

1. Fork the repository and create a branch from `main`.
2. Keep a pull request focused on one thing. Small pull requests are reviewed faster.
3. Format Rust code with `cargo fmt` (settings in `rustfmt.toml`).
4. Run the PC tests. If your change affects the firmware, test it on a real board and say which board and chip revision you used.
5. Add a line to [CHANGELOG.md](CHANGELOG.md) for user-visible changes.
6. Fill in the pull request template.

### Code notes

- Source code, comments and file names are partly in German. You may write new code and comments in English or German; please stay consistent within a file.
- Original KolibriOS C programs should run unchanged. Fix compatibility problems in the Rust system functions, not in the KolibriOS programs.
- Memory is tight (internal RAM, 10 MB for the browser). Mention memory impact in the pull request if your change allocates a lot.
- Shell scripts must use LF line endings (enforced by `.gitattributes`).

## Licensing

ESP32-OS is licensed under the GNU General Public License v2.0 ([LICENSE](LICENSE)); the `spatz` crate is MIT OR Apache-2.0. By submitting a pull request you agree that your contribution is published under the license of the part of the repository you change. If you add third-party code, include its license text in [`LICENSES/`](LICENSES/) and list it in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Code of conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md). By participating you agree to respect it.
