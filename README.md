# Omvix Hub

> **Lightweight, open-source, pure-Rust hardware control center for HP Victus and HP OMEN laptops — with zero OMEN Gaming Hub dependency.**
>
> *HP Victus ve HP OMEN dizüstü bilgisayarlar için OMEN Gaming Hub gerektirmeyen, hafif, açık kaynaklı ve tamamen Rust tabanlı donanım kontrol merkezi.*

---

## Key Features / Özellikler

- **Zero OGH Dependency / OGH Bağımsızlığı:** Communicates directly with HP ACPI/WMI firmware (`root\WMI\hpqBIntM`, `hpqBEvnt`) without Microsoft Store apps, background telemetry, or HP HSA services.
- **Victus & OMEN Support / Victus ve OMEN Desteği:** Auto-detects board ID (`Win32_BaseBoard.Product`, e.g., Victus 16 `8C99`) and selects the appropriate thermal profile protocol (`OmenV0`, `OmenV1`, `Victus`, `VictusS`).
- **Thermal & Fan Control:** Performance mode switching (`Eco`, `Quiet`, `Balanced`, `Performance`), `Auto`/`Max`/`Manual` speed, interactive `CustomCurve` with downward hysteresis, and a 2-minute keep-alive guard preventing Victus/OMEN BIOS timeout resets.
- **Battery Care / Batarya Sağlığı:** Native WMI `0x2B` charge limiting (`Auto`, `Inhibit`, `Hold`), custom upper/lower thresholds, **one-time 100% travel charge**, wear %, capacity, and charge/discharge wattage (inspired by `hp-charge-limit-rs` & `hp-charge-limiter`).
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

# Build optimized standalone release binaries (~7.6 MB GUI, ~2.7 MB CLI)
cargo build --release --workspace

# Launch Omvix Hub GUI (click "Restart as Administrator" inside Dashboard for WMI BIOS writes)
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
