pub mod battery;
pub mod diagnostics;
pub mod home;
pub mod hotkeys;
pub mod lighting_display;
pub mod monitor;
pub mod performance;
pub mod settings;
pub mod tuning;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    Monitor,
    PerformanceFans,
    Battery,
    LightingDisplay,
    Tuning,
    Hotkeys,
    Diagnostics,
    Settings,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[
            Self::Home,
            Self::Monitor,
            Self::PerformanceFans,
            Self::Battery,
            Self::LightingDisplay,
            Self::Tuning,
            Self::Hotkeys,
            Self::Diagnostics,
            Self::Settings,
        ]
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Home => "nav.home",
            Self::Monitor => "nav.monitor",
            Self::PerformanceFans => "nav.performance_fans",
            Self::Battery => "nav.battery",
            Self::LightingDisplay => "nav.lighting_display",
            Self::Tuning => "nav.tuning",
            Self::Hotkeys => "nav.hotkeys",
            Self::Diagnostics => "nav.diagnostics",
            Self::Settings => "nav.settings",
        }
    }
}
