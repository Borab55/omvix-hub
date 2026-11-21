use omvix_core::{OmvixError, Result, WindowsPowerOverlay};
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::core::{GUID, PCSTR, s, w};

type PowerSetActiveOverlaySchemeFn = unsafe extern "system" fn(*const GUID) -> u32;
type PowerGetActualOverlaySchemeFn = unsafe extern "system" fn(*mut GUID) -> u32;

pub fn guid_for_overlay(overlay: WindowsPowerOverlay) -> GUID {
    GUID::from_u128(overlay.guid_u128())
}

pub fn overlay_from_guid(guid: GUID) -> Option<WindowsPowerOverlay> {
    if guid == guid_for_overlay(WindowsPowerOverlay::BestPowerEfficiency) {
        Some(WindowsPowerOverlay::BestPowerEfficiency)
    } else if guid == guid_for_overlay(WindowsPowerOverlay::Balanced) {
        Some(WindowsPowerOverlay::Balanced)
    } else if guid == guid_for_overlay(WindowsPowerOverlay::BestPerformance) {
        Some(WindowsPowerOverlay::BestPerformance)
    } else {
        None
    }
}

struct PowrProfHandle {
    module: HMODULE,
    loaded_dynamically: bool,
}

impl Drop for PowrProfHandle {
    fn drop(&mut self) {
        if self.loaded_dynamically && !self.module.is_invalid() {
            unsafe {
                let _ = FreeLibrary(self.module);
            }
        }
    }
}

unsafe fn acquire_powrprof() -> Result<PowrProfHandle> {
    if let Ok(module) = unsafe { GetModuleHandleW(w!("powrprof.dll")) } {
        if !module.is_invalid() {
            return Ok(PowrProfHandle {
                module,
                loaded_dynamically: false,
            });
        }
    }
    let module = unsafe { LoadLibraryW(w!("powrprof.dll")) }
        .map_err(|e| OmvixError::UnsupportedHardware(format!("LoadLibraryW(powrprof.dll): {e}")))?;
    Ok(PowrProfHandle {
        module,
        loaded_dynamically: true,
    })
}

unsafe fn resolve_proc(module: HMODULE, name: PCSTR) -> Option<*const ()> {
    unsafe { GetProcAddress(module, name) }.map(|p| p as *const ())
}

pub fn set_windows_power_overlay(overlay: WindowsPowerOverlay) -> Result<()> {
    let handle = unsafe { acquire_powrprof()? };
    let proc_ptr = unsafe { resolve_proc(handle.module, s!("PowerSetActiveOverlayScheme")) }
        .ok_or_else(|| {
            OmvixError::UnsupportedHardware(
                "PowerSetActiveOverlayScheme not exported by powrprof.dll".to_string(),
            )
        })?;
    let func: PowerSetActiveOverlaySchemeFn = unsafe { std::mem::transmute(proc_ptr) };
    let guid = guid_for_overlay(overlay);
    let rc = unsafe { func(&guid) };
    if rc == 0 {
        tracing::info!(?overlay, "Synchronized Windows Power Mode overlay");
        Ok(())
    } else {
        Err(OmvixError::UnsupportedHardware(format!(
            "PowerSetActiveOverlayScheme failed with status {rc}"
        )))
    }
}

pub fn read_windows_power_overlay() -> Option<WindowsPowerOverlay> {
    let handle = unsafe { acquire_powrprof() }.ok()?;
    let proc_ptr = unsafe { resolve_proc(handle.module, s!("PowerGetActualOverlayScheme")) }?;
    let func: PowerGetActualOverlaySchemeFn = unsafe { std::mem::transmute(proc_ptr) };
    let mut guid = GUID::zeroed();
    let rc = unsafe { func(&mut guid) };
    if rc == 0 {
        overlay_from_guid(guid)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_guid_fields_and_roundtrip_match_windows_constants() {
        let eff = guid_for_overlay(WindowsPowerOverlay::BestPowerEfficiency);
        assert_eq!(eff.data1, 0x961cc777);
        assert_eq!(eff.data2, 0x2547);
        assert_eq!(eff.data3, 0x4f9d);
        assert_eq!(eff.data4, [0x81, 0x74, 0x7d, 0x86, 0x18, 0x1b, 0x8a, 0x7a]);
        assert_eq!(
            overlay_from_guid(eff),
            Some(WindowsPowerOverlay::BestPowerEfficiency)
        );

        let bal = guid_for_overlay(WindowsPowerOverlay::Balanced);
        assert_eq!(bal, GUID::zeroed());
        assert_eq!(overlay_from_guid(bal), Some(WindowsPowerOverlay::Balanced));

        let perf = guid_for_overlay(WindowsPowerOverlay::BestPerformance);
        assert_eq!(perf.data1, 0xded574b5);
        assert_eq!(perf.data2, 0x45a0);
        assert_eq!(perf.data3, 0x4f42);
        assert_eq!(
            perf.data4,
            [0x87, 0x37, 0x46, 0x34, 0x5c, 0x09, 0xc2, 0x38]
        );
        assert_eq!(
            overlay_from_guid(perf),
            Some(WindowsPowerOverlay::BestPerformance)
        );

        let unknown = GUID::from_u128(0x11111111_2222_3333_4444_555555555555);
        assert_eq!(overlay_from_guid(unknown), None);
    }
}
