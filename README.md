# Omvix Hub

> **Lightweight, open-source, pure-Rust hardware control center for HP Victus and HP OMEN laptops — with zero OMEN Gaming Hub dependency.**
>
> *HP Victus ve HP OMEN dizüstü bilgisayarlar için OMEN Gaming Hub gerektirmeyen, hafif, açık kaynaklı ve tamamen Rust tabanlı donanım kontrol merkezi.*

---

## Key Features / Özellikler

- **Zero OGH Dependency / OGH Bağımsızlığı:** Communicates directly with HP ACPI/WMI firmware (`root\WMI\hpqBIntM`, `hpqBEvnt`) without Microsoft Store apps, background telemetry, or HP HSA services.
- **Victus & OMEN Support / Victus ve OMEN Desteği:** Auto-detects board ID (`Win32_BaseBoard.Product`, e.g., Victus 16 `8C99`) and selects the appropriate thermal profile protocol (`OmenV0`, `OmenV1`, `Victus`, `VictusS`).
- **Thermal & Fan Control:** Performance mode switching (`Eco`, `Quiet`, `Balanced`, `Performance`), `Auto`/`Max`/`Manual` speed, interactive `CustomCurve` with downward hysteresis, and a 2-minute keep-alive guard preventing Victus/OMEN BIOS timeout resets.
- **Dual-Tier Telemetry & Windows EMI:** Supports **0.1s–5.0s** live polling of CPU %/Clock, **CPU Package Power (W via Windows EMI — works under VBS/HVCI)**, NVIDIA NVML (Temp, Load, Power/TGP Limit, VRAM, Clocks, P-State, Throttle), Disk & Network I/O, Session Min/Max/Avg, and 3 dedicated real-time plots (Temperature, Usage, Power), while keeping EC/WMI BIOS reads safely capped at `1.0s`.
- **Battery Care / Batarya Sağlığı:** Native WMI `0x2B` charge limiting (`Auto`, `Inhibit`, `Hold`), one-click presets (`60% / 80% / 90% / 100%`), unlimited custom battery profiles, **one-time 100% travel charge**, wear %, capacity, voltage, remaining time, and charge/discharge wattage.
- **G-Helper-Style Smart Cockpit & Neutral Charcoal Theme:** Opens as a compact bottom-right `456 × 676` cockpit with tactile mode tiles, and expands smoothly to `1060 × 700` when opening `Graphs`, `Fans & Power`, `Battery`, or `Extra` drawers — with optional auto-hide on focus loss and neutral **Dark Graphite** (`#161618`) default styling.
- **Low-Power iGPU UI & Shareable Themes:** Built on `eframe` + `wgpu` (`PowerPreference::LowPower`) to render cleanly on integrated graphics without waking dGPU, with 5 presets, live RGB pickers, and Copy/Paste shareable theme codes (`OMVIX1|...` / JSON).
- **Display, Lighting, GPU & CPU Tuning:** 1-zone/4-zone RGB & white backlight control, GPU MUX switch (`Hybrid`/`Discrete`/`Optimus`), display refresh rate (`Hz`) switcher, AC/Battery auto-profiles, CPU Turbo Boost, PL1/PL2/TCC/Undervolt controls (with automatic VBS/HVCI lock detection).
- **Hotkeys & System Tray:** Dedicated OMEN key capture (`hpqBEvnt`) with custom action/command assignment, global toggle shortcut (`Ctrl+Shift+O`), minimize-to-tray on close, and tray quick actions.
- **i18n & Diagnostics:** Instant **Türkçe / English** switching (extensible JSON packs under `locales/`), companion/conflict service detector, JSON diagnostic export, and timestamped ring-rotating file logger capped at **20 MB** (`4 x 5 MB`).

---

## Workspace Structure

```text
hp-omen-hub-lite/
├── crates/
│   ├── core/              # omvix-core: OS-agnostic models, traits, fan curve, battery care, auto-profile, i18n, 20 MB logger
│   ├── backend-windows/   # omvix-windows: COM/WMI hpqBIntM & hpqBEvnt, NVML, sysinfo, Win32 Power/GDI/Registry
│   ├── backend-linux/     # omvix-linux: Linux sysfs/ACPI trait scaffold (hp-wmi, platform_profile, hwmon, acpi_call)
│   ├── cli/               # omvix-cli: Hardware probe, telemetry, display, battery, thermal/fan CLI
│   └── app/               # omvix-hub: Pure Rust desktop GUI (egui/eframe + egui_plot + tray-icon)
├── locales/               # en.json, tr.json
├── docs/                  # Architecture & protocol notes
└── CHANGELOG.md
```

---

## Build & Run

```powershell
# Run all workspace unit tests
cargo test --workspace

# Build optimized standalone release binaries (~12.3 MB GUI w/ wgpu, ~2.9 MB CLI)
cargo build --release --workspace

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
