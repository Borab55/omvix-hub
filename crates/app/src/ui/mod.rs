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
    Cpu,
    Gpu,
    Profiles,
}

impl FansPowerSubTab {
    pub fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Gpu,
            Self::Gpu => Self::Profiles,
            Self::Profiles => Self::Cpu,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Cpu => Self::Profiles,
            Self::Gpu => Self::Cpu,
            Self::Profiles => Self::Gpu,
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

pub fn default_desktop_or_temp_file(filename: &str) -> std::path::PathBuf {
    #[cfg(windows)]
    if let Some(desktop) = std::env::var_os("USERPROFILE") {
        let candidate = std::path::PathBuf::from(desktop).join("Desktop");
        if candidate.exists() {
            return candidate.join(filename);
        }
    }
    std::env::temp_dir().join(filename)
}

pub const REFRESH_OPTIONS_MS: &[(u64, &str)] = &[
    (100, "0.1s"),
    (250, "0.25s"),
    (500, "0.5s"),
    (1000, "1s"),
    (1500, "1.5s"),
    (2000, "2s"),
    (5000, "5s"),
];

pub fn open_folder_in_explorer(dir: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new(omvix_windows::paths::system_tool("explorer.exe"))
            .arg(dir)
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = dir;
}

pub fn unique_profile_slug(name: &str, fallback: &str, is_used: impl Fn(&str) -> bool) -> String {
    let slug_base = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if slug_base.is_empty() {
        fallback.to_string()
    } else {
        slug_base
    };
    let mut candidate = base.clone();
    let mut n = 2usize;
    while is_used(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
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

        assert_eq!(FansPowerSubTab::Cpu.next(), FansPowerSubTab::Gpu);
        assert_eq!(FansPowerSubTab::Profiles.next(), FansPowerSubTab::Cpu);
        assert_eq!(FansPowerSubTab::Cpu.prev(), FansPowerSubTab::Profiles);

        assert_eq!(ExtraSubTab::LightingDisplay.next(), ExtraSubTab::Hotkeys);
        assert_eq!(ExtraSubTab::Settings.next(), ExtraSubTab::LightingDisplay);
        assert_eq!(ExtraSubTab::LightingDisplay.prev(), ExtraSubTab::Settings);
    }
}
