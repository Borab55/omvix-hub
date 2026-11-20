# Changelog

All notable changes to **Omvix Hub** will be documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/en.json/1.1.0/).

## [0.3.3] - 2026-11-20

### Fixed
- **Windows Startup Hotfix (`main.rs`):** Restored `wgpu`'s default enabled backends (`Vulkan` via `PowerPreference::LowPower`) by removing the `Backends::DX12` override (`eframe 0.31`'s default `wgpu` feature set does not compile `wgpu/dx12`, which caused `Failed to create surface for any enabled backend`).

## [0.3.2] - 2026-11-20

### Fixed & Added (Critical Thermal Safety, Sensor Clarity & UI Polish)
- **Critical Fan Stall Fix (`thermal_fan.rs`, `telemetry.rs`, `wmi_bios.rs`, `core/thermal.rs`):**
  - Removed the dangerous `0x20008, 0x2E [0, 0]` write on `FanMode::Auto` (on HP Victus `8C99` / OMEN firmware, writing level `0` stops the fans completely at `0 RPM` for 120 seconds instead of restoring BIOS control). `FanMode::Auto` now clears Max Fan (`0x27 [0]`) and re-applies the active performance profile via `0x1A` with `byBios = 0x01`.
  - Stopped passive telemetry/diagnostics from calling `0x20008, 0x10` (`FanCount`), which was arming the firmware's 120-second user-defined fan timer even in Auto mode. Telemetry now discovers fan count from `FanTableGet (0x2F)`.
  - Added `MIN_SAFE_FAN_LEVEL = 18` (~1800 RPM floor) in `safe_manual_wmi_level` and added `ThermalSafetyGuard` (`CPU >= 90°C`, `GPU >= 86°C`, `Chassis >= 54°C`, or `0 RPM` under load for 2 consecutive EC polls -> emergency `FanMode::Max` with hysteresis release).
- **True CPU Die vs. EC Chassis Sensor Separation + PawnIO `IntelMSR` Reader (`pawnio.rs`, `telemetry.rs`, `monitor.rs`, `cockpit.rs`):**
  - Stopped mislabeling HP EC `0x20008, 0x23` (chassis/IR sensor, e.g. `46°C`) as CPU die temperature. `0x23` is now stored separately in `TelemetrySnapshot::chassis_temp_c`.
  - Added optional zero-dependency Win32 `DeviceIoControl` client for signed **PawnIO** + `IntelMSR.bin` (`MSR_IA32_TEMPERATURE_TARGET` `0x1A2` + `MSR_IA32_PACKAGE_THERM_STATUS` `0x1B1` with median-of-3 filtering) to read true CPU package die temperature even under Win11 VBS/HVCI.
  - Added `effective_fan_curve_temp_c` so `CustomCurve` safely bridges GPU temp, conservative chassis mapping, and EMI CPU package power when PawnIO is not installed.
- **UI & Graphics Improvements (`battery.rs`, `cockpit.rs`, `monitor.rs`, `performance.rs`, `main.rs`, `logging.rs`):**
  - Added `pill_button` borders to all built-in/custom battery profiles, hold-mode buttons, and travel toggles.
  - Disabled plot mouse-wheel interception (`.allow_scroll(false)`, `.allow_boxed_zoom(false)`, `.allow_double_click_reset(false)`) on all `Monitor` and `Performance & Fans` charts so outer page scrolling works smoothly.
  - Increased window height slightly (`468 × 712` <-> `1080 × 712`) so custom battery profiles don't crowd Card A, switched Windows `wgpu` backend to `Backends::DX12`, and silenced noisy `wgpu_hal` warnings in `omvix.log`.

## [0.3.1] - 2026-11-20

### Fixed & Improved
- **Compact Cockpit Overflow Calibration (`468 × 680` <-> `1080 × 680`):**
  - Locked cockpit interior content width to `pane_width - 24` and card interior width to `content_w - 22`, computing all tile widths from `card_inner_w` so zero elements can overflow past the right card/window boundary.
  - Added short cockpit-specific tile labels (`cockpit.fan_*`: `Auto / Max / Manual / Curve`, `Oto / Maks / Manuel / Eğri`; `cockpit.boost_*`: `Off / Enabled / Aggressive`, `Kapalı / Etkin / Agresif`; compact `{hz}Hz` when 5+ modes are present), tighter `tile_button` padding (`4×2`) with `.truncate()`, and tighter card margins (`10×6`) so all 5 cards (including **Battery Charge Limit**) fit cleanly without scrolling or clipping.
- **Non-Resizable Fixed Cockpit Frame:**
  - Disabled user border drag-resizing and removed the maximize button (`.with_resizable(false).with_maximize_button(false)`).
- **Right-Anchored Leftward Drawer Expansion:**
  - Pinned the compact cockpit on the **right** (`SidePanel::right`) and opened detail drawers on the **left** (`CentralPanel`), paired with an atomic Win32 right-edge-anchored resize (`win32_resize_anchored_right`) and updated `Compact >>` / `Daralt >>` collapse button so bottom-right docked windows expand smoothly to the left without shifting rightward.

## [0.3.0] - 2026-11-20

### Added & Changed
- **G-Helper-Style Smart-Expanding Cockpit (`feat/v0.3-ghelper-expanding-cockpit`):**
  - Redesigned main window to a compact **456 × 676 px** quick-control cockpit (`ui/cockpit.rs`) with live CPU/GPU/Fan/RAM/Battery strip, full-width rounded mode tiles (`tile_button`), 1-click Display Hz, CPU Boost, Keyboard RGB swatch/brightness, Battery threshold presets + `1x %100` travel toggle, and sticky bottom drawer buttons (`Graphs`, `Fans & Power`, `Battery`, `Extra`).
  - Added smart horizontal window expansion (**456 × 676** <-> **1060 × 700**) opening the selected drawer on the right while staying bottom-right anchored (`win32_dock_bottom_right`).
  - Added optional `"Anchor window to bottom-right (G-Helper style)"` (`dock_bottom_right`, enabled by default) and `"Auto-hide to tray when clicking outside (Compact mode)"` (`hide_on_focus_loss`) toggles in Settings (`config.rs`, `settings.rs`).
- **Neutral Charcoal Default Theme (`Dark Graphite`):**
  - Replaced bluish dark slate (`#0f141b`) with a neutral charcoal-graphite palette (`#161618` canvas / `#1F1F22` cards / `#343439` borders) and automatic migration for existing default `v0.2.0` configs (`is_legacy_bluish_dark_slate`).

## [0.2.0] - 2026-11-20

### Added & Calibrated
- **Hardware & Firmware Calibrations (verified on Victus 16-r1025nt `8C99` BIOS `F.19`):**
  - Replaced `FanMaxGet (0x26)` (actually Max-Fan boolean flag) with `FanTableGet (0x2F)` parser (`HpWmiBios::parse_max_fan_levels_from_table`) scaling manual/curve levels to actual fan table maxima (`0x3A` / `0x3D` = `5800` / `6100 RPM`).
  - Added WMI keyboard type `0x07` mapping to `SingleZoneRgb` with board-database fallback (`keyboard.rs`).
  - Calibrated `decode_charge_mode` for `0x2B` (`04 FF 01 00` -> `Inhibit`) and added battery voltage (V), remaining capacity (mWh), and estimated remaining time (`battery.rs`).
  - Removed erroneous `0x21` GPU power fallback in `gpu_mux.rs` and surfaced clean `UnsupportedHardware` guidance when `0x52`/`0x53` returns `RC=3`.
- **Windows EMI CPU Package Power & Dual-Tier Telemetry (`emi.rs`, `telemetry.rs`):**
  - Added Windows Energy Meter Interface (`GUID_DEVICE_ENERGY_METER` / `IOCTL_EMI_*`) reader for live CPU Package Power (Watts) working natively under Windows 11 VBS/HVCI with zero kernel drivers.
  - Added dual-tier polling supporting `0.1s` (`100 ms`) up to `5.0s` UI refresh intervals on fast sensors (CPU %, clock, CPU EMI power, RAM, NVML GPU load/temp/power/TGP limit/VRAM/core & mem clocks/P-state/throttle reasons, Disk R/W MB/s, Net Down/Up MB/s) while capping EC/WMI BIOS & `BatteryStatus` polls safely at `>= 1.0s`.
- **Rendering, Tray & Window Reliability (`wgpu` + Win32 `HWND`):**
  - Switched `eframe` from `glow` (OpenGL) to `wgpu` with `PowerPreference::LowPower` (selecting Intel UHD iGPU / DX12 instead of waking the RTX dGPU).
  - Fixed hidden-window freeze on tray/Omen-key restore & quit via direct Win32 `ShowWindow`/`SetForegroundWindow`/`PostMessageW(WM_CLOSE)`, tray immediate proxy commands, and a safe quit watchdog.
  - Added single-instance mutex (`Local\OmvixHubSingleInstance`) restoring existing window, plus clean hotkey/mutex release before UAC admin restart.
  - Added `ThreadWmiSessionGuard` (`HpWmiBios::enable_thread_cache` + persistent `root\WMI` connection) eliminating repeated COM/WMI reconnect overhead and log spam.
  - Added instant synchronous Frame-1 Admin status detection (`is_process_elevated`) and 4-phase incremental startup discovery.
- **UI & UX Upgrades (`Home` Cockpit, `Monitor`, Battery Profiles, Custom Shareable Themes):**
  - Split `Dashboard` into **Home (`Giriş Paneli`)** (compact KPI cards + 1-click Thermal, Fan, Battery Profile, Display Hz, Keyboard Light, CPU Boost cockpit) and **System Monitor (`Sistem İzleme`)** (`0.1s`–`5s` refresh controls, Pause/Reset, Session Min/Max/Avg, Disk/Net/VRAM metrics, and 3 separate plots: Temperature `°C`, Usage `%`, Power `W`).
  - Added 4 built-in Battery Charge Presets (`Desktop 60%`, `Balanced 80%`, `Travel 90%`, `Full 100%`) and full Custom Battery Profile CRUD.
  - Fixed sidebar active tab losing rounded corners (`CornerRadius::same(8)` across all states + custom `sidebar_tab_button`).
  - Added custom theme color pickers, 5 built-in presets, and Copy/Paste theme sharing (`OMVIX1|...` compact codes & JSON), plus full EN/TR localization parity.

## [0.1.0] - 2026-11-20

### Added
- Repository initialized with `.gitignore`, `README.md`, `CHANGELOG.md`, and `docs/ARCHITECTURE.md`.
- Established Git workflow (`main` <- `dev` <- `feat/*`).
- **Phase 0 (`feat/project-foundation`):**
  - Created Cargo workspace (`omvix-core`, `omvix-windows`, `omvix-linux`, `omvix-cli`, `omvix-hub`).
  - Added `omvix-core`: unified errors, cross-platform backend traits, fan curve evaluator with downward hysteresis, board database (`8C99` Victus 16-r1xxx and OMEN/Victus models), TOML atomic config persistence, JSON localization (`en.json` & `tr.json`), and timestamped ring-rotating file logger strictly capped at 20 MB (`4 x 5 MB`) with panic hook.
  - Added `omvix-linux` backend scaffold targeting `hp-wmi` and `acpi_call`.
- **Phase 1 (`feat/wmi-bios-probe`):**
  - Implemented native Windows COM/WMI client (`wmi_com.rs`, `wmi_bios.rs`) for `root\WMI\hpqBIntM` with `SECU`-signed BIOS mailbox calls (`hpqBIOSInt0..4096`) and a 15-point strictly read-only probe suite.
  - Added system identity, `8C99` board matching, `Win32_DeviceGuard` VBS/HVCI detection, and companion/conflict service scanner (`HPOmenCap`, `hp-charge-limit-rs`, `XTU3SERVICE`, etc.) in `system_info.rs`.
  - Built `omvix-cli` with `probe [--verbose]`, `export-diagnostics`, and `logs-path` subcommands.
- **Phase 2 (`feat/telemetry-monitor`):**
  - Implemented `WindowsTelemetryBackend` using `sysinfo` (CPU usage/clock, RAM), `nvml-wrapper` (NVIDIA GPU name, temperature, load, power, graphics clock), Win32 `GetSystemPowerStatus`, `root\WMI` `BatteryStatus` & `MSAcpi_ThermalZoneTemperature`, and optional HP WMI fan RPMs.
  - Added `omvix-cli telemetry` subcommand and verified live telemetry on Victus `16-r1025nt` (`RTX 4070`).
- **Phase 3 (`feat/thermal-fan-control`):**
  - Added board-aware thermal profile mapping (`VictusS` / `OmenV1` `0x30`/`0x31`/`0x50` and legacy schemes) and fan level scaling in `omvix-core::thermal`.
  - Implemented `WindowsThermalFanBackend` (`set_performance_mode`, `set_fan_mode`, and 2-minute EC timeout keep-alive tick) via native WMI `hpqBIntM`.
  - Added `omvix-cli thermal set` and `omvix-cli fan set` CLI commands.
- **Phase 4 (`feat/battery-charge-limit`):**
  - Implemented `BatteryCareEngine` in `omvix-core` with upper/lower threshold hysteresis, hold mode, and automatic one-time 100% travel charge reversion.
  - Implemented `WindowsBatteryBackend` (`0x2B` WMI charge limit read/write, design/full capacity, wear %, and charge/discharge rate in Watts).
  - Added `omvix-cli battery status` and `omvix-cli battery set-mode` commands; verified live battery health & rate readout on Victus `16-r1025nt`.
- **Phase 5 (`feat/keyboard-gpu-tuning-hotkeys`):**
  - Implemented `AutoProfileEngine` in `omvix-core` for AC/DC automatic thermal/refresh/keyboard transitions.
  - Implemented `WindowsKeyboardBackend` (`0x20009` backlight power & 1-zone/4-zone RGB), `WindowsGpuSwitchBackend` (MUX `0x52`/`0x53`), `WindowsDisplayBackend` (GDI refresh rate enumeration & switching), `WindowsCpuTuningBackend` (Win32 Power Plan Boost mode + WMI/PawnIO/VBS guards for PL1/PL2/TCC/Undervolt), `hotkeys` (`hpqBEvnt` Omen key + `RegisterHotKey` global toggle), and `autostart` (`HKCU Run`).
  - Added `omvix-cli display list` and `omvix-cli tuning status` commands; verified live `1920x1080` (`60..144 Hz`) and `Aggressive` CPU Boost status on Victus `16-r1025nt`.
- **Phase 6 (`feat/modern-ui-tray`):**
  - Implemented modern pure-Rust `egui`/`eframe` desktop GUI (`omvix-hub`) with Dark Slate & Clean Light themes, live Turkish/English switching, real-time `egui_plot` temperature/load charts, interactive fan curve editor, battery care + travel charge controls, RGB + display Hz + MUX switch panel, CPU tuning + VBS/HVCI lock notice, Omen key + global shortcut settings, hardware diagnostics table + JSON export, and 20 MB log viewer/controls.
  - Added `tray-icon` integration supporting minimize-to-tray on close, tray mode/fan/battery toggles, and clean exit from tray, plus a 1-click "Restart as Administrator" trigger (`ShellExecuteW("runas")`).
  - Built standalone portable release binaries (`omvix-hub.exe` ~7.57 MB, `omvix-cli.exe` ~2.72 MB) relying only on built-in Windows DLLs.
