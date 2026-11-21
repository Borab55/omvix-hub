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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawer {
    Monitor,
    FansPower,
    Battery,
    Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FansPowerSubTab {
    #[default]
    PerformanceFans,
    Profiles,
    Tuning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExtraSubTab {
    #[default]
    LightingDisplay,
    Hotkeys,
    Diagnostics,
    Settings,
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

pub fn default_support_bundle_path() -> std::path::PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("omvix-support-{stamp}.zip");
    #[cfg(windows)]
    if let Some(desktop) = std::env::var_os("USERPROFILE") {
        let candidate = std::path::PathBuf::from(desktop).join("Desktop");
        if candidate.exists() {
            return candidate.join(&filename);
        }
    }
    std::env::temp_dir().join(filename)
}

pub fn show_file_in_folder(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if path.exists() {
            let safe = path.to_string_lossy().replace('"', "");
            let _ = std::process::Command::new("explorer.exe")
                .raw_arg(format!("/select,\"{safe}\""))
                .spawn();
        } else if let Some(parent) = path.parent() {
            let _ = std::process::Command::new("explorer.exe")
                .arg(parent)
                .spawn();
        }
    }
    #[cfg(not(windows))]
    let _ = path;
}
