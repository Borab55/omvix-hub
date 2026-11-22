# Omvix Hub

[![CI](https://github.com/Borab55/omvix-hub/actions/workflows/ci.yml/badge.svg)](https://github.com/Borab55/omvix-hub/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Borab55/omvix-hub?include_prereleases&label=release)](https://github.com/Borab55/omvix-hub/releases)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)](#)

> **Lightweight, open-source, pure-Rust hardware control center for HP Victus and HP OMEN laptops — with zero OMEN Gaming Hub dependency.**
>
> *HP Victus ve HP OMEN dizüstü bilgisayarlar için OMEN Gaming Hub gerektirmeyen, hafif, açık kaynaklı ve tamamen Rust tabanlı donanım kontrol merkezi.*

---

## Download / İndir

- Download standalone binaries (`omvix-hub.exe`, `omvix-cli.exe`, `omvix-hub-windows-x64.zip`, `SHA256SUMS.txt`) from [**GitHub Releases**](https://github.com/Borab55/omvix-hub/releases) — single-file executables with zero external or MinGW DLL dependencies.
- **Note / Not:** Current pre-release binaries do not carry an Authenticode code-signing certificate yet (Windows SmartScreen may show a warning; verify downloads against `SHA256SUMS.txt`). Launch `omvix-hub.exe` and click **Restart as Administrator** when BIOS/WMI writes are required.

---

## Key Features / Özellikler

- **Zero OGH Dependency / OGH Bağımsızlığı:** Communicates directly with HP ACPI/WMI firmware (`root\WMI\hpqBIntM`, `hpqBEvnt`) without Microsoft Store apps, background telemetry, or HP HSA services.
- **Victus & OMEN Support / Victus ve OMEN Desteği:** Auto-detects board ID (`Win32_BaseBoard.Product`, e.g., Victus 16 `8C99`) and selects the appropriate thermal profile protocol (`OmenV0`, `OmenV1`, `Victus`, `VictusS`).
- **Thermal & Fan Control:** Performance mode switching (`Eco`, `Quiet`, `Balanced`, `Performance`), `Auto`/`Max`/`Manual` speed, interactive 8-point `CustomCurve` with presets, downward hysteresis & safe share codes, configurable sustained Thermal Guard, Sensor-Loss safety fallback, and Safe Mode (`--safe` / startup `Shift` / 3-launch crash-loop guard).
- **Dual-Tier Telemetry, CSV Export & Windows EMI:** Supports **0.1s–5.0s** live polling of CPU %/Clock, **CPU Package Power (W via Windows EMI)**, signed **PawnIO (`IntelMSR.bin`)** CPU die temperature, zero-wake NVIDIA **D3 sleep detection** + NVML stabilization, Disk & Network I/O, Session Min/Max/Avg, native `egui::Painter` real-time area charts, and 600-row CSV export while keeping EC/WMI reads safely capped at `1.0s`.
- **Battery Care & Weekly Health Chart:** Native WMI `0x2B` charge limiting (`Auto`, `Inhibit`, `Hold`) with explicit `Unknown` firmware-read handling, presets (`60% / 80% / 90% / 100%`), custom profiles, **one-time 100% travel charge** with persistent deadline/auto-clear, built-in `OmvixBatteryService` for AC/sleep/shutdown enforcement, and persistent weekly battery health & wear trend chart.
- **AC/DC & Per-App `.exe` Profiles:** Automatic AC/DC fan & curve switching, per-application foreground `.exe` profiles with debounce & baseline restore, and automatic Windows Power Mode synchronization.
- **Zero-Flicker Dual-Viewport Cockpit & Accessibility:** Compact stationary bottom-right cockpit with a left-side detail viewport (`Graphs`, `Fans & Power`, `Battery`, `Extra`), quick detail-drawer buttons, permanent compact **Auto AC/DC** cockpit toggle, 5-step onboarding wizard, **High Contrast** preset + `%85–%130` font scaling, keyboard navigation (`Ctrl+1..4`, `Ctrl+Tab`, `Alt+Left/Right`, `Esc`), and a **12-color custom theme library** with share codes (`OMVIX2|...`, backward-compatible with `OMVIX1`) built on low-power `eframe` + `wgpu` (`PowerPreference::LowPower`).
- **Display, Lighting, GPU & CPU Tuning:** 1-zone/4-zone RGB & white backlight control with custom 4-zone lighting profiles, 2-step GPU MUX switch (`Hybrid`/`Discrete`/`Optimus`), display refresh rate (`Hz`) switcher, CPU Turbo Boost, and PL1/PL2 controls (with VBS/HVCI detection and automatic BIOS default restoration on disable/reset).
- **Hotkeys, System Tray & Signed Updater:** OMEN key capture (`hpqBEvnt`) with unelevated `.exe`/`.lnk` custom commands, global toggle shortcut with simultaneous auto-capture & Win32 conflict detection, mode-colored tray icon + quick submenus + OSD/Toast alerts, COM in-memory elevated autostart, System32 DLL/binary path hardening, and a pinned `Borab55/omvix-hub` GitHub Releases updater with mandatory `SHA256SUMS.txt` & Authenticode verification.
- **10-Step Undo, Support Bundle & i18n:** 10-step hardware change undo history, 1-click redacted `.zip` support bundle export, corrupt `config.toml` quarantine backup (`.bad-*`), instant **Türkçe / English** localization (`locales/`), conflict service detector, and 20 MB rotating logs (`4 × 5 MB`).

---

## Workspace Structure

```text
omvix-hub/
├── .github/workflows/     # ci.yml (CI & DLL audit), release.yml (automated GitHub Releases)
├── crates/
│   ├── core/              # omvix-core: OS-agnostic models, traits, fan curve, battery care/history, profiles, undo, zip bundle, updater, i18n
│   ├── backend-windows/   # omvix-windows: COM/WMI hpqBIntM & hpqBEvnt, NVML, PawnIO, EMI, SCM battery service, Win32 Power/Tray/Updater
│   ├── backend-linux/     # omvix-linux: Linux sysfs/ACPI trait scaffold (hp-wmi, platform_profile, hwmon, acpi_call)
│   ├── cli/               # omvix-cli: Hardware probe, telemetry, display, battery, thermal/fan CLI
│   └── app/               # omvix-hub: Pure Rust desktop GUI (eframe/wgpu + native egui::Painter charts + tray-icon)
├── locales/               # en.json, tr.json
├── docs/                  # Architecture, protocol notes & code signing report
└── CHANGELOG.md
```

---

## Build & Run

```powershell
# Run all workspace unit tests
cargo test --workspace --locked

# Build optimized standalone release binaries (~13.6 MB GUI w/ wgpu, ~3.0 MB CLI)
cargo build --release --workspace --locked

# Launch Omvix Hub GUI (click "Restart as Administrator" inside Home cockpit for WMI BIOS writes)
.\target\release\omvix-hub.exe

# Run smoke test
.\target\release\omvix-hub.exe --smoke-test
```

### CLI Examples (`omvix-cli.exe`)

```powershell
# Read-only hardware & WMI probe (run as Administrator to inspect hpqBIntM hex responses)
.\target\release\omvix-cli.exe probe --verbose

# Export JSON diagnostic report
.\target\release\omvix-cli.exe export-diagnostics --out diagnostics.json

# Live telemetry, battery health, display modes, CPU boost/VBS status
.\target\release\omvix-cli.exe telemetry
.\target\release\omvix-cli.exe battery status
.\target\release\omvix-cli.exe display list
.\target\release\omvix-cli.exe tuning status
.\target\release\omvix-cli.exe logs-path

# Elevated control commands (Administrator terminal)
.\target\release\omvix-cli.exe thermal set performance
.\target\release\omvix-cli.exe fan set manual --speed 65
.\target\release\omvix-cli.exe battery set-mode inhibit
```

### Paths (Windows)
- **Config:** `%APPDATA%\OmvixHub\config.toml`
- **Rotating Logs (20 MB cap):** `%LOCALAPPDATA%\OmvixHub\logs\omvix.log`

---

## Recovery / Kurtarma

If Omvix Hub was deleted manually while **Battery Hold (`0x0300`)** or **Inhibit (`0x0100`)** was active, the HP firmware (`0x2B`) retains that charge limit persistently even across reboots or Windows shutdowns.

*Omvix Hub klasörü batarya **Hold (`0x0300`)** veya **Inhibit (`0x0100`)** aktifken elle silindiyse, BIOS/EC firmware bu şarj kısıtını yeniden başlatma veya kapatma sonrasında da kalıcı olarak tutmaya devam eder.*

To restore normal 100% battery charging at any time (**no GUI needed**):

```powershell
# Run in an Administrator terminal / Yönetici terminalinde çalıştırın:
.\omvix-cli.exe battery auto
```

Alternatively (without `omvix-cli.exe`), reboot into **HP BIOS Setup (`F10`)** and load **Restore Defaults**, or perform a CMOS/EC reset by holding the power button for 15–30 seconds while unplugged.
