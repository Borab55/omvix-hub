pub mod battery;
pub mod cockpit;
pub mod diagnostics;
pub mod hotkeys;
pub mod lighting_display;
pub mod monitor;
pub mod performance;
pub mod settings;
pub mod tuning;

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
