# Changelog

All notable changes to **Omvix Hub** will be documented in this file.
Format follows [Keep a Changelog](https://keepachangelog.com/en.json/1.1.0/).

## [0.4.4] - 2026-11-21

### Added & Changed
- **Instant Commit-on-Release Controls & Redundant Apply Button Cleanup (`ui/mod.rs`, `cockpit.rs`, `performance.rs`, `battery.rs`, `lighting_display.rs`, `settings.rs`, `hotkeys.rs`):**
  - Added `control_committed` helper so sliders, drag values, and color pickers apply hardware commands and save settings once upon pointer release or keyboard commit — never flooding WMI/EC or disk on every drag frame.
  - Removed redundant `Apply` buttons for safe/live controls (cockpit & drawer Manual Fan Speed, Keyboard Lighting brightness & RGB colors, Cockpit & Battery Charge Limit sliders/toggles/presets, Custom Fan Curve edits, Hysteresis, Theme color pickers, Log Max Size, and Auto-Profile AC/Battery Hz combos).
  - Preserved safety-critical and validation-gated `Apply` buttons: CPU Power Limits (PL1/PL2), TCC Temperature Limit, Core Undervolt, Global Shortcut (plus Enter-to-apply when valid), and the non-admin UAC elevation button.
- **3-Layer SSD & Hardware Write Protection (`config.rs`, `battery_service.rs`, `main.rs`, `worker.rs`):**
  - **Identical-content disk skip:** `AppConfig::save_to_path` and `BatteryServiceConfig::save_to_path` compare serialized TOML against existing files on disk and skip temporary-file write + rename when unchanged.
  - **400 ms UI config debounce:** Interactive config writes batch within a 400 ms window (flushed immediately on hide-to-tray, UAC restart, quit, or factory reset).
  - **Worker command coalescing & redundant WMI skip:** `coalesce_worker_commands` deduplicates burst latest-wins hardware commands while respecting lifecycle barriers, `UpdateConfig` skips `OmvixBatteryService` disk sync unless battery fields changed and immediately re-evaluates `CustomCurve` points, and `ApplyBatteryNow` skips `0x2B` WMI writes when firmware already matches the target charge mode.

## [0.4.3] - 2026-11-21

### Added & Improved
- **Native Sleep, Modern Standby (S0ix) & Windows Shutdown Safety (`power_events.rs`, `worker.rs`, `main.rs`):**
  - Added hidden Win32 top-level window listening for `WM_QUERYENDSESSION` / `WM_ENDSESSION` and registered via `RegisterSuspendResumeNotification` for Modern Standby (`PBT_APMSUSPEND`) and Wake (`PBT_APMRESUME*`) broadcasts.
  - Synchronously restores `FanMode::Auto` (preventing stuck `0 RPM` or fixed speeds when closing lid or shutting down) and applies `Battery Hold (0x0300)` (or `Auto` if disabled / below 40%) before Windows suspends or shuts down; automatically restores active Performance Mode, Fan Mode/Curve, and Keyboard Lighting on resume.
- **Hidden-Tray Battery & Idle Optimizations (`telemetry.rs`, `worker.rs`):**
  - Throttles background telemetry to `>= 1.0s` while hidden in system tray and skips heavy Disk/Network `sysinfo` refreshes when the UI is not visible.
  - Keeps idle Optimus RTX dGPUs asleep (`D3cold`) by skipping NVML polling/initialization whenever the window is hidden on battery (`DC`).
  - Added automatic 30s cooldown retry (capped at 3/5 attempts) for `PawnIO` (`IntelMSR.bin`) and `NVML` initialization so early Windows startup races self-heal automatically.

## [0.4.2] - 2026-11-21

### Added & Changed
- **Shutdown Charge Hold Default On (`config.rs`, `worker.rs`):**
  - Changed `BatteryConfig::default().hold_when_shutdown` to `true` and added `AppConfig::config_version = 2` so pre-v0.4.2 configs automatically migrate `hold_when_shutdown` to `true` once without overriding explicit user toggles afterwards.
  - When `OmvixBatteryService` is not installed and `hold_when_shutdown == true`, quitting Omvix Hub cleanly applies `Hold (0x0300)` instead of resetting to `Auto` (as long as battery `>= 40%` and `>= start_threshold_pct`).
- **Native Pure-Rust Windows Battery Background Service (`battery_service.rs`, `main.rs`, `worker.rs`, `battery.rs`):**
  - Added built-in Windows SCM service mode (`OmvixBatteryService` via `--battery-service` flag in `omvix-hub.exe`, installed cleanly to `C:\Program Files\OmvixHub\omvix-battery-svc.exe` with config & logs under `C:\ProgramData\OmvixHub`).
  - Handles `SERVICE_CONTROL_STOP`, `SERVICE_CONTROL_PRESHUTDOWN`, and `SERVICE_CONTROL_POWEREVENT` (`PBT_APMSUSPEND` / `PBT_APMRESUME*`) synchronously so `Hold (0x0300)` is written before sleep/hibernate/shutdown (when above `off_min_pct = 40%` and `start_threshold_pct`), and hot-reloads threshold/mode changes made in the UI within 2 seconds.
  - Added full service status, visible binary/data folder paths with 1-click **Open Folder** buttons, Install/Update/Start/Stop/Restart actions, and 2-step **Zero-Trace Uninstall** in the **Battery Care** tab (stops/deletes SCM service, restores `0x2B Auto`, removes both `C:\Program Files\OmvixHub` and `C:\ProgramData\OmvixHub`, also wired into **Restore All Defaults**).

## [0.4.1] - 2026-11-21

### Added & Improved
- **Smart Thermal Guard (`< 1800 RPM` / `>= 90°C`) & Cool-Idle 0-RPM Support (`core/thermal.rs`, `thermal_fan.rs`, `worker.rs`, `charts.rs`):**
  - Added `is_system_cool_for_zero_rpm` and `smart_manual_wmi_level`: when sensors are cool (`CPU < 65°C`, `GPU < 60°C`, `Chassis < 46°C`), manual and curve modes allow true `0%` (0 RPM) and `< 1800 RPM` quiet speeds instead of unconditionally clamping to `1800 RPM`. When warm or sensors are missing, `MIN_SAFE_FAN_LEVEL` (`18`) is enforced.
  - Upgraded `ThermalSafetyGuard` to engage **Emergency Max Fan** immediately at `CPU >= 90°C`, `GPU >= 86°C`, or `Chassis >= 54°C`, or on 2 consecutive polls where any fan runs `< 1800 RPM` while under thermal load (`CPU >= 78°C`, `GPU >= 74°C`, `Chassis >= 48°C`, or `CPU Power >= 55W`), releasing cleanly only after 5 consecutive cool polls (`<= 78°C / 74°C / 46°C` and all fans `>= 1800 RPM`).
  - Fixed downward curve hysteresis baseline bug in `worker.rs` so slow 1°C/step cooling properly tracks peak baseline and transitions downward smoothly once `hysteresis_c` is cleared.
- **Clean Exit Safety Restores (`worker.rs`, `battery.rs`, `main.rs`):**
  - On clean application **Quit** (`WorkerCommand::Shutdown`), restores `FanMode::Auto` whenever Manual/Curve/Max or Emergency Guard was active, avoiding a 120-second wait on fixed speeds or 0 RPM after exit.
  - Wired `battery.hold_when_shutdown` into `battery.rs` UI and `worker.rs` shutdown: when unchecked (default), quitting Omvix Hub cleanly restores `0x2B Auto` normal charging (while Hiding to tray keeps the charge limit active).
- **Auto-Admin Elevation & UAC-Free Startup (`autostart.rs`, `main.rs`, `settings.rs`):**
  - Automatically requests UAC elevation (`ShellExecuteW("runas", "--no-elevate")`) on interactive non-admin launch, falling back gracefully to non-admin telemetry mode if declined (and never prompting during `--smoke-test`).
  - Upgraded **Start with Windows** (`autostart.rs`) to register an elevated **Windows Task Scheduler** (`OmvixHub`, `RunLevel = HighestAvailable`, battery-safe, `PT0S` unlimited execution) when running as Admin — starting Omvix Hub as Administrator on login with **zero UAC popup** (and removing both scheduled task + legacy `HKCU\Run` when disabled).
- **UI & Header Polish (`cockpit.rs`, `theme.rs`, `performance.rs`, `locales/en.json`, `locales/tr.json`):**
  - Removed redundant green `Admin` badge when elevated; kept clickable amber `Admin ?` badge only when not elevated.
  - Replaced plain `! N` conflict counter with a custom-painted vector **warning triangle badge** (`clickable_warning_badge`).
  - Added live RAM usage percentage (`RAM X.X/16G (XX%)`) in the cockpit telemetry strip.
  - Moved fan Keep-Alive & Hysteresis controls with inline explanations into a collapsible **Advanced Fan Settings** section in `Performance & Fans`.

## [0.4.0] - 2026-11-20

### Added & Improved
- **Zero-Flicker Dual-Viewport Drawer & Exact DWM Bottom-Right Lock (`main.rs`, `app/Cargo.toml`):**
  - Replaced single-window resize expansion with a secondary `ctx.show_viewport_immediate` detail window anchored strictly to the **left** of the stationary compact cockpit (`468 × 728` cockpit + `632 × 728` drawer), eliminating frame stretch/jitter on open/close.
  - Switched `win32_dock_bottom_right` to `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` and enforced active position lock while visible so the visible frame sits cleanly in the bottom-right corner without Win11 shadow margins.
- **Native `egui::Painter` Area Charts & Direct-Drag 8-Point Fan Curve (`charts.rs`, `performance.rs`, `monitor.rs`, `models.rs`):**
  - Replaced `egui_plot` widgets with custom high-DPI `egui::Painter` telemetry area charts (translucent gradient fills, vertical hover crosshairs, floating value pills) and zero scroll-wheel interception.
  - Added direct touch/mouse drag-and-drop **8-point fan curve editor** (`35°C..=95°C`) with monotonic temperature clamping, live operating-point vertical indicator, `Min 1800 RPM` floor visualization, and floating `°C -> % (~RPM)` readout.
- **1-Click Signed PawnIO Installer (`pawnio.rs`, `telemetry.rs`, `monitor.rs`, `settings.rs`, `worker.rs`):**
  - Added background installer downloading `PawnIO_setup.exe` (Authenticode signature verified) + `IntelMSR.bin` module directly into `%LOCALAPPDATA%\OmvixHub\pawnio` and hot-reloading CPU package die temperature without restarting.
- **UI Polish & Factory Defaults Reset (`cockpit.rs`, `settings.rs`, `worker.rs`):**
  - Increased cockpit bottom footer vertical spacing (`10 px` gap between drawer tiles and `Gizle` / `Çıkış` row).
  - Added 2-step **Restore All Defaults** action in Settings resetting BIOS Auto fans, Balanced mode, 100% normal charging (`0x2B Auto`), Windows Aggressive Boost, max native Hz, white keyboard backlight, Windows autostart removal, and `config.toml` defaults.

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
