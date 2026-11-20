# Omvix Hub — Architecture & Technical Design

## 1. Design Principles

1. **Zero OMEN Gaming Hub (OGH) Dependency:** All hardware discovery, thermal policies, fan controls, keyboard lighting, MUX switching, and battery charge limiting talk directly to HP's ACPI/WMI BIOS routines without requiring Microsoft Store packages or HP background HSA services.
2. **Board-Aware Capability Discovery:** On startup, Omvix Hub identifies the motherboard (`Win32_BaseBoard.Product`, e.g., `8C99` for HP Victus `16-r1xxx`) and probes read-only WMI endpoints individually so unsupported capabilities are gracefully hidden rather than causing errors.
3. **Windows-First, Linux-Ready:** All domain models, configuration, i18n, logging, fan curve math, and backend traits live in `omvix-core`. Platform specifics are isolated behind trait implementations in `omvix-windows` and `omvix-linux`.
4. **Security & Modern Windows Compatibility:** No insecure `WinRing0` driver. WMI and standard Win32/NVML APIs handle the vast majority of features driver-free. Direct CPU MSR tuning (undervolt `0x150`, TCC `0x1A2`, PL1/PL2 `0x610`) checks `Win32_DeviceGuard` VBS/HVCI state first and uses an optional signed `PawnIO` bridge only when unlocked.

---

## 2. Cargo Workspace Layout

| Crate | Package Name | Responsibility |
|---|---|---|
| `crates/core` | `omvix-core` | Domain models, backend traits, board database, fan curve evaluator with hysteresis, TOML config, JSON i18n, 20 MB ring-rotating logger |
| `crates/backend-windows` | `omvix-windows` | COM/WMI `hpqBIntM` client, `hpqBEvnt` listener, system identity & conflict service scanner, VBS/HVCI check, NVML & Win32 helpers |
| `crates/backend-linux` | `omvix-linux` | Trait scaffolding targeting `hp-wmi` (`/sys/firmware/acpi/platform_profile`, `hwmon`) and `acpi_call` (`\_SB.WMID.SBCC`) |
| `crates/cli` | `omvix-cli` | Read-only hardware probe, diagnostic JSON exporter, log inspector, and CLI controls |
| `crates/app` | `omvix-hub` | Pure Rust `egui`/`eframe` desktop GUI, background keep-alive/curve loop, and `tray-icon` integration |

---

## 3. Windows WMI BIOS Protocol (`root\WMI\hpqBIntM`)

HP OMEN and Victus laptops expose an ACPI WMI mailbox under namespace `root\WMI`:

- **Interface Class:** `hpqBIntM`
- **Methods:** `hpqBIOSInt0`, `hpqBIOSInt4`, `hpqBIOSInt128`, `hpqBIOSInt1024`, `hpqBIOSInt4096` (suffix indicates expected response `Data` buffer size in bytes)
- **Input Instance (`hpqBDataIn`):**
  - `Sign` (`UInt8Array`): 4-byte ASCII `"SECU"` (`[0x53, 0x45, 0x43, 0x55]`)
  - `Command` (`UInt32`): Primary command group (`0x20008` Gaming/Thermal/System, `0x20009` Keyboard/RGB, `0x00001` Standard BIOS)
  - `CommandType` (`UInt32`): Sub-opcode (e.g., `0x1A` performance mode, `0x2B` battery charge control, `0x2D`/`0x2E`/`0x2F` Victus-S fan queries/sets)
  - `Size` (`UInt32`): Input payload byte length
  - `hpqBData` (`UInt8Array`): Command payload bytes
- **Output Instance (`hpqBDataOut{N}`):**
  - `rwReturnCode` (`UInt32`): `0` = OK, `3` = Unknown/Unsupported Command, `5` = Invalid Parameter/Buffer Size
  - `Data` (`UInt8Array`): Response payload
- **Event Class:** `hpqBEvnt` (`EventID`, `EventData`) used to capture Omen key presses.
- **Privilege Requirement:** Windows requires Administrator privileges to query/invoke `root\WMI\hpqBIntM`.

---

## 4. Logging Architecture (20 MB Cap)

- **Format:** `YYYY-MM-DD HH:MM:SS.mmm | LEVEL | target | message`
- **Ring Rotation:** Uses `4 x 5 MB` rotating files (`omvix.log`, `omvix.1.log`, `omvix.2.log`, `omvix.3.log`) by default, guaranteeing total log footprint never exceeds `max_total_mb` (default **20 MB**) while preserving recent diagnostic history across rollovers.
- **Panic Hook:** Unhandled panics are flushed directly to the log file before process termination.

---

## 5. VBS / HVCI & i7-14700HX Undervolt Reality

Even on unlocked HX CPUs (`i7-14700HX`), Windows 11 Memory Integrity (HVCI / `VirtualizationBasedSecurityStatus = 2`) blocks third-party ring-0 drivers from modifying `MSR 0x150` voltage offsets. Omvix Hub detects this state via `Win32_DeviceGuard`, informs the user clearly in the Tuning tab, and keeps WMI/Win32 thermal, fan, power-limit, boost, and battery controls fully operational regardless of VBS state.

---

## 6. Linux Roadmap (`omvix-linux`)

Future Linux implementation will map the exact same `omvix-core` traits onto:
- **Thermal Profiles:** `/sys/firmware/acpi/platform_profile` (`hp-wmi`)
- **Fan RPM / PWM:** `/sys/class/hwmon/hwmon*/fan*_input` & `pwm*`
- **Battery Charge Limit:** `\_SB.WMID.SBCC` via `acpi_call` or kernel battery charge threshold sysfs
- **GPU & Display:** `supergfxctl` / `envycontrol` / X11-Wayland refresh rate APIs
