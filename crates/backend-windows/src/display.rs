use omvix_core::{DisplayBackend, DisplayModeInfo, OmvixError, Result};
use windows::core::PCWSTR;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
    DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplaySettingsW, CDS_UPDATEREGISTRY, DEVMODEW,
    DISP_CHANGE_RESTART, DISP_CHANGE_SUCCESSFUL, DM_BITSPERPEL, DM_DISPLAYFREQUENCY, DM_PELSHEIGHT,
    DM_PELSWIDTH, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE,
};

/// Windows display refresh rate enumerator and switcher via Win32 GDI + DisplayConfig (CCD).
#[derive(Debug, Default)]
pub struct WindowsDisplayBackend;

impl WindowsDisplayBackend {
    pub fn new() -> Self {
        Self
    }

    fn current_devmode(device_name: PCWSTR) -> Result<DEVMODEW> {
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let ok = unsafe { EnumDisplaySettingsW(device_name, ENUM_CURRENT_SETTINGS, &mut mode) };
        if !ok.as_bool() {
            return Err(OmvixError::UnsupportedHardware(
                "EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS) failed".to_string(),
            ));
        }
        Ok(mode)
    }

    fn query_source_gdi_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
        let mut req = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        let rc = unsafe { DisplayConfigGetDeviceInfo(&mut req.header) };
        if rc != ERROR_SUCCESS.0 as i32 {
            return None;
        }
        let end = req
            .viewGdiDeviceName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(req.viewGdiDeviceName.len());
        let name = String::from_utf16_lossy(&req.viewGdiDeviceName[..end]);
        if name.trim().is_empty() {
            None
        } else {
            Some(name)
        }
    }

    /// Resolves the target active display's GDI device name (`\\.\DISPLAYx`) and CCD hardware Hz.
    /// Prefers the built-in laptop panel (`INTERNAL` / `DISPLAYPORT_EMBEDDED`) so multi-GPU/hybrid
    /// topologies (e.g. Intel iGPU @ 144 Hz + dGPU virtual @ 60 Hz) never misreport the secondary path.
    fn resolve_target_display() -> Option<(Vec<u16>, Option<u32>)> {
        let mut paths = Vec::<DISPLAYCONFIG_PATH_INFO>::new();
        let mut modes = Vec::<DISPLAYCONFIG_MODE_INFO>::new();

        for _ in 0..3 {
            let mut num_paths = 0u32;
            let mut num_modes = 0u32;
            let err = unsafe {
                GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut num_paths, &mut num_modes)
            };
            if err != ERROR_SUCCESS || num_paths == 0 {
                return None;
            }
            paths.resize(num_paths as usize, DISPLAYCONFIG_PATH_INFO::default());
            modes.resize(num_modes as usize, DISPLAYCONFIG_MODE_INFO::default());
            let qerr = unsafe {
                QueryDisplayConfig(
                    QDC_ONLY_ACTIVE_PATHS,
                    &mut num_paths,
                    paths.as_mut_ptr(),
                    &mut num_modes,
                    modes.as_mut_ptr(),
                    None,
                )
            };
            if qerr == ERROR_SUCCESS {
                paths.truncate(num_paths as usize);
                break;
            }
            if qerr != ERROR_INSUFFICIENT_BUFFER {
                return None;
            }
        }

        let mut best: Option<(i32, String, Option<u32>)> = None;
        for p in &paths {
            let Some(gdi_name) = Self::query_source_gdi_name(p) else {
                continue;
            };
            let tech = p.targetInfo.outputTechnology;
            let is_internal = tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED;
            let score = if is_internal { 2 } else { 1 };
            let r = p.targetInfo.refreshRate;
            let hz = (r.Denominator > 0)
                .then(|| (f64::from(r.Numerator) / f64::from(r.Denominator)).round() as u32)
                .filter(|&v| v > 0);
            if best.as_ref().is_none_or(|(b_score, _, _)| score > *b_score) {
                best = Some((score, gdi_name, hz));
            }
        }

        best.map(|(_, gdi_name, hz)| {
            let mut wide: Vec<u16> = gdi_name.encode_utf16().collect();
            wide.push(0);
            (wide, hz)
        })
    }

    pub(crate) fn reconcile_refresh_rate(
        gdi_hz: u32,
        ccd_hz: Option<u32>,
        supported_hz: &[u32],
    ) -> u32 {
        match ccd_hz {
            Some(hw) if hw > 0 => supported_hz
                .iter()
                .copied()
                .filter(|&s| s.abs_diff(hw) <= 1)
                .min_by_key(|&s| s.abs_diff(hw))
                .unwrap_or(hw),
            _ => gdi_hz,
        }
    }
}

impl DisplayBackend for WindowsDisplayBackend {
    fn get_display_mode(&self) -> Result<DisplayModeInfo> {
        let resolved = Self::resolve_target_display();
        let (dev_ptr, ccd_hz) = match resolved.as_ref() {
            Some((wide, hz)) => (PCWSTR(wide.as_ptr()), *hz),
            None => (PCWSTR::null(), None),
        };

        let current = Self::current_devmode(dev_ptr)?;
        let width = current.dmPelsWidth;
        let height = current.dmPelsHeight;
        let bpp = current.dmBitsPerPel;

        let mut supported_hz = Vec::new();
        let mut index = 0u32;
        loop {
            let mut candidate = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            let ok = unsafe {
                EnumDisplaySettingsW(dev_ptr, ENUM_DISPLAY_SETTINGS_MODE(index), &mut candidate)
            };
            if !ok.as_bool() {
                break;
            }
            if candidate.dmPelsWidth == width
                && candidate.dmPelsHeight == height
                && candidate.dmBitsPerPel == bpp
                && candidate.dmDisplayFrequency > 0
            {
                supported_hz.push(candidate.dmDisplayFrequency);
            }
            index += 1;
        }

        supported_hz.sort_unstable();
        supported_hz.dedup();

        let current_hz =
            Self::reconcile_refresh_rate(current.dmDisplayFrequency, ccd_hz, &supported_hz);

        Ok(DisplayModeInfo {
            current_hz,
            supported_hz,
            width,
            height,
        })
    }

    fn set_refresh_rate(&self, hz: u32) -> Result<()> {
        let resolved = Self::resolve_target_display();
        let dev_ptr = match resolved.as_ref() {
            Some((wide, _)) => PCWSTR(wide.as_ptr()),
            None => PCWSTR::null(),
        };

        let mut mode = Self::current_devmode(dev_ptr)?;
        let width = mode.dmPelsWidth;
        let height = mode.dmPelsHeight;
        let bpp = mode.dmBitsPerPel;

        let mut matched_mode: Option<DEVMODEW> = None;
        let mut index = 0u32;
        loop {
            let mut candidate = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            let ok = unsafe {
                EnumDisplaySettingsW(dev_ptr, ENUM_DISPLAY_SETTINGS_MODE(index), &mut candidate)
            };
            if !ok.as_bool() {
                break;
            }
            if candidate.dmPelsWidth == width
                && candidate.dmPelsHeight == height
                && candidate.dmBitsPerPel == bpp
                && candidate.dmDisplayFrequency == hz
            {
                matched_mode = Some(candidate);
                break;
            }
            index += 1;
        }

        if let Some(m) = matched_mode {
            mode = m;
            mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_BITSPERPEL | DM_DISPLAYFREQUENCY;
        } else {
            mode.dmDisplayFrequency = hz;
            mode.dmFields = DM_DISPLAYFREQUENCY;
        }

        let res = unsafe {
            ChangeDisplaySettingsExW(
                dev_ptr,
                Some(&mode),
                HWND::default(),
                CDS_UPDATEREGISTRY,
                None,
            )
        };
        if res == DISP_CHANGE_SUCCESSFUL || res == DISP_CHANGE_RESTART {
            tracing::info!(hz, "Changed display refresh rate");
            Ok(())
        } else {
            Err(OmvixError::UnsupportedHardware(format!(
                "ChangeDisplaySettingsExW({hz} Hz) returned error code {}",
                res.0
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WindowsDisplayBackend;

    #[test]
    fn reconcile_prefers_ccd_hw_rate_and_snaps_fractional_rates() {
        let supported = [60, 75, 100, 120, 144];
        // Stale GDI 60 Hz when real panel runs at 144 Hz -> returns 144
        assert_eq!(
            WindowsDisplayBackend::reconcile_refresh_rate(60, Some(144), &supported),
            144
        );
        // Fractional CCD rate 143 / 59 snaps cleanly to supported integer mode
        assert_eq!(
            WindowsDisplayBackend::reconcile_refresh_rate(60, Some(143), &supported),
            144
        );
        assert_eq!(
            WindowsDisplayBackend::reconcile_refresh_rate(144, Some(59), &supported),
            60
        );
        // Missing CCD rate falls back cleanly to GDI rate
        assert_eq!(
            WindowsDisplayBackend::reconcile_refresh_rate(120, None, &supported),
            120
        );
    }
}
