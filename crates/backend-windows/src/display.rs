use omvix_core::{DisplayBackend, DisplayModeInfo, OmvixError, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplaySettingsW, CDS_UPDATEREGISTRY, DEVMODEW,
    DISP_CHANGE_RESTART, DISP_CHANGE_SUCCESSFUL, DM_DISPLAYFREQUENCY, ENUM_CURRENT_SETTINGS,
    ENUM_DISPLAY_SETTINGS_MODE,
};

/// Windows display refresh rate enumerator and switcher via Win32 GDI.
#[derive(Debug, Default)]
pub struct WindowsDisplayBackend;

impl WindowsDisplayBackend {
    pub fn new() -> Self {
        Self
    }

    fn current_devmode() -> Result<DEVMODEW> {
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let ok = unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut mode) };
        if !ok.as_bool() {
            return Err(OmvixError::UnsupportedHardware(
                "EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS) failed".to_string(),
            ));
        }
        Ok(mode)
    }
}

impl DisplayBackend for WindowsDisplayBackend {
    fn get_display_mode(&self) -> Result<DisplayModeInfo> {
        let current = Self::current_devmode()?;
        let width = current.dmPelsWidth;
        let height = current.dmPelsHeight;
        let bpp = current.dmBitsPerPel;
        let current_hz = current.dmDisplayFrequency;

        let mut supported_hz = Vec::new();
        let mut index = 0u32;
        loop {
            let mut candidate = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            let ok = unsafe {
                EnumDisplaySettingsW(
                    PCWSTR::null(),
                    ENUM_DISPLAY_SETTINGS_MODE(index),
                    &mut candidate,
                )
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

        Ok(DisplayModeInfo {
            current_hz,
            supported_hz,
            width,
            height,
        })
    }

    fn set_refresh_rate(&self, hz: u32) -> Result<()> {
        let mut mode = Self::current_devmode()?;
        mode.dmDisplayFrequency = hz;
        mode.dmFields = DM_DISPLAYFREQUENCY;

        let res = unsafe {
            ChangeDisplaySettingsExW(
                PCWSTR::null(),
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
