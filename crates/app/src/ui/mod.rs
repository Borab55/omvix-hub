pub mod battery;
pub mod charts;
pub mod cockpit;
pub mod diagnostics;
pub mod hotkeys;
pub mod lighting_display;
pub mod monitor;
pub mod performance;
pub mod profiles;
pub mod settings;
pub mod tuning;
pub mod wizard;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Drawer {
    Monitor,
    FansPower,
    Battery,
    Extra,
}

impl Drawer {
    pub fn next(self) -> Self {
        match self {
            Self::Monitor => Self::FansPower,
            Self::FansPower => Self::Battery,
            Self::Battery => Self::Extra,
            Self::Extra => Self::Monitor,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Monitor => Self::Extra,
            Self::FansPower => Self::Monitor,
            Self::Battery => Self::FansPower,
            Self::Extra => Self::Battery,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FansPowerSubTab {
    #[default]
    PerformanceFans,
    Profiles,
    Tuning,
}

impl FansPowerSubTab {
    pub fn next(self) -> Self {
        match self {
            Self::PerformanceFans => Self::Profiles,
            Self::Profiles => Self::Tuning,
            Self::Tuning => Self::PerformanceFans,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::PerformanceFans => Self::Tuning,
            Self::Profiles => Self::PerformanceFans,
            Self::Tuning => Self::Profiles,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExtraSubTab {
    #[default]
    LightingDisplay,
    Hotkeys,
    Diagnostics,
    Settings,
}

impl ExtraSubTab {
    pub fn next(self) -> Self {
        match self {
            Self::LightingDisplay => Self::Hotkeys,
            Self::Hotkeys => Self::Diagnostics,
            Self::Diagnostics => Self::Settings,
            Self::Settings => Self::LightingDisplay,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::LightingDisplay => Self::Settings,
            Self::Hotkeys => Self::LightingDisplay,
            Self::Diagnostics => Self::Hotkeys,
            Self::Settings => Self::Diagnostics,
        }
    }
}

/// Returns `true` once after a continuous control (`Slider`, `DragValue`, `color_edit_button_srgb`)
/// changes value and the user finishes pointer interaction (mouse release) or commits via keyboard.
/// Prevents 60 Hz WMI/EC hardware calls and SSD writes while dragging.
pub fn control_committed(
    ui: &egui::Ui,
    id_salt: impl std::hash::Hash,
    resp: &egui::Response,
) -> bool {
    let dirty_id = ui.make_persistent_id(id_salt).with("omvix_commit_dirty");
    let was_dirty: bool = ui.ctx().data_mut(|d| d.get_temp(dirty_id)).unwrap_or(false);
    let dirty = was_dirty || resp.changed();
    let actively_dragging = resp.dragged() || ui.input(|i| i.pointer.any_down());

    if actively_dragging {
        if dirty {
            ui.ctx().data_mut(|d| d.insert_temp(dirty_id, true));
        }
        false
    } else if dirty {
        ui.ctx().data_mut(|d| d.remove::<bool>(dirty_id));
        true
    } else {
        false
    }
}

fn default_desktop_or_temp_file(filename: &str) -> std::path::PathBuf {
    #[cfg(windows)]
    if let Some(desktop) = std::env::var_os("USERPROFILE") {
        let candidate = std::path::PathBuf::from(desktop).join("Desktop");
        if candidate.exists() {
            return candidate.join(filename);
        }
    }
    std::env::temp_dir().join(filename)
}

pub fn default_support_bundle_path() -> std::path::PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    default_desktop_or_temp_file(&format!("omvix-support-{stamp}.zip"))
}

pub fn default_telemetry_csv_path() -> std::path::PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    default_desktop_or_temp_file(&format!("omvix-telemetry-{stamp}.csv"))
}

pub fn show_file_in_folder(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if path.exists() {
            let safe = path.to_string_lossy().replace('"', "");
            let _ = std::process::Command::new(omvix_windows::paths::system_tool("explorer.exe"))
                .raw_arg(format!("/select,\"{safe}\""))
                .spawn();
        } else if let Some(parent) = path.parent() {
            let _ = std::process::Command::new(omvix_windows::paths::system_tool("explorer.exe"))
                .arg(parent)
                .spawn();
        }
    }
    #[cfg(not(windows))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawer_and_subtab_cycles_wrap_cleanly() {
        assert_eq!(Drawer::Monitor.next(), Drawer::FansPower);
        assert_eq!(Drawer::Extra.next(), Drawer::Monitor);
        assert_eq!(Drawer::Monitor.prev(), Drawer::Extra);
        assert_eq!(Drawer::FansPower.prev(), Drawer::Monitor);

        assert_eq!(
            FansPowerSubTab::PerformanceFans.next(),
            FansPowerSubTab::Profiles
        );
        assert_eq!(
            FansPowerSubTab::Tuning.next(),
            FansPowerSubTab::PerformanceFans
        );
        assert_eq!(
            FansPowerSubTab::PerformanceFans.prev(),
            FansPowerSubTab::Tuning
        );

        assert_eq!(ExtraSubTab::LightingDisplay.next(), ExtraSubTab::Hotkeys);
        assert_eq!(ExtraSubTab::Settings.next(), ExtraSubTab::LightingDisplay);
        assert_eq!(ExtraSubTab::LightingDisplay.prev(), ExtraSubTab::Settings);
    }
}
