pub mod battery;
pub mod dashboard;
pub mod diagnostics;
pub mod hotkeys;
pub mod lighting_display;
pub mod performance;
pub mod settings;
pub mod tuning;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
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
            Self::Dashboard,
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
            Self::Dashboard => "nav.dashboard",
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
