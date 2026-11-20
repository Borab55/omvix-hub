# Omvix Hub

> **Lightweight, open-source, pure-Rust hardware control center for HP Victus and HP OMEN laptops — with zero OMEN Gaming Hub dependency.**
>
> *HP Victus ve HP OMEN dizüstü bilgisayarlar için OMEN Gaming Hub gerektirmeyen, hafif, açık kaynaklı ve tamamen Rust tabanlı donanım kontrol merkezi.*

---

## Key Goals / Temel Hedefler

- **Zero OGH Dependency / OGH Bağımsızlığı:** Communicates directly with HP ACPI/WMI firmware (`root\WMI\hpqBIntM`, `hpqBEvnt`) without Microsoft Store apps, background telemetry, or HP HSA services.
- **Victus & OMEN Support / Victus ve OMEN Desteği:** Auto-detects board ID (`Win32_BaseBoard.Product`, e.g., Victus 16 `8C99`) and selects the appropriate thermal profile protocol (`V0`, `V1`, `Victus`, `Victus-S`).
- **Battery Care / Batarya Sağlığı:** Native WMI `0x2B` charge limiting (`Auto`, `Inhibit`, `Hold`), custom upper/lower thresholds, one-time 100% travel charge, wattage and wear reporting (inspired by `hp-charge-limit-rs` & `hp-charge-limiter`).
- **Thermal & Fan Control:** Performance mode switching, Auto/Max/Manual/Custom curve with hysteresis and anti-2-minute-reset keep-alive loop.
- **Display, Lighting, GPU & CPU Tuning:** 1-zone/4-zone RGB & white backlight control, MUX switch (`Hybrid`/`Discrete`), refresh rate (`Hz`) switcher, AC/Battery auto-profiles, CPU boost & PL1/PL2/TCC/Undervolt tuning (with automatic VBS/HVCI lock detection), Omen key capture & global hotkeys.
- **Clean Architecture / Temiz Mimari:** Windows-first Cargo workspace with OS-agnostic `omvix-core` traits and a Linux backend scaffold (`omvix-linux` targeting `hp-wmi`, `hwmon`, `acpi_call`).
- **Observability & i18n:** Ring-rotating timestamped logging strictly capped at **20 MB** (`4 x 5 MB`), plus **English & Turkish** localization ready for community language packs.

---

## Workspace Structure

```text
hp-omen-hub-lite/
├── crates/
│   ├── core/              # omvix-core: OS-agnostic models, traits, fan curve engine, board DB, config, i18n, 20 MB rotating logger
│   ├── backend-windows/   # omvix-windows: Native COM/WMI hpqBIntM client, system diagnostics, NVML, Win32, optional PawnIO bridge
│   ├── backend-linux/     # omvix-linux: Linux sysfs/ACPI trait scaffold (hp-wmi, platform_profile, hwmon, acpi_call SBCC)
│   ├── cli/               # omvix-cli: Safe hardware probe, diagnostic JSON export, and CLI testing tool
│   └── app/               # omvix-hub: Pure Rust desktop GUI (egui/eframe) + System Tray
├── locales/               # en.json, tr.json (extensible JSON localization files)
├── docs/                  # Architecture notes, WMI protocol reference, board compatibility matrix
└── CHANGELOG.md
```

---

## Quick Start (Development)

```powershell
# Run workspace tests
cargo test --workspace

# Run read-only hardware diagnostics on an HP laptop (run in elevated terminal for root\WMI access)
cargo run -p omvix-cli -- probe --verbose

# Export JSON diagnostic snapshot
cargo run -p omvix-cli -- export-diagnostics --out diagnostics.json
```

> **Note:** On Windows, accessing `root\WMI\hpqBIntM` requires Administrator privileges.
