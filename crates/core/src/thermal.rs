use crate::boards::lookup_board;
use crate::models::{PerformanceMode, ThermalProfileScheme};

/// Default maximum WMI fan level step when firmware `FanMaxGet` (0x26) is unavailable.
pub const DEFAULT_MAX_FAN_LEVEL: u8 = 55;

/// Maps an abstract `PerformanceMode` and board `ThermalProfileScheme` to the HP BIOS WMI byte.
pub fn thermal_profile_byte(scheme: ThermalProfileScheme, mode: PerformanceMode) -> u8 {
    match scheme {
        ThermalProfileScheme::VictusS | ThermalProfileScheme::OmenV1 => match mode {
            PerformanceMode::Eco | PerformanceMode::Quiet => 0x50,
            PerformanceMode::Balanced => 0x30,
            PerformanceMode::Performance => 0x31,
        },
        ThermalProfileScheme::Victus
        | ThermalProfileScheme::OmenV0
        | ThermalProfileScheme::Unknown => match mode {
            PerformanceMode::Balanced => 0x00,
            PerformanceMode::Performance => 0x01,
            PerformanceMode::Eco | PerformanceMode::Quiet => 0x02,
        },
    }
}

/// Converts a 0..=100 percentage into a WMI fan level step (0..=max_level).
pub fn fan_percent_to_wmi_level(speed_pct: u8, max_level: u8) -> u8 {
    let pct = speed_pct.clamp(0, 100) as f32;
    let max_l = max_level.max(1) as f32;
    ((pct / 100.0) * max_l).round().clamp(0.0, max_l) as u8
}

/// Determines the most likely `ThermalProfileScheme` from board ID and model/family strings.
pub fn resolve_scheme_from_identity(
    board_id: &str,
    model: &str,
    family: &str,
) -> ThermalProfileScheme {
    if let Some(board) = lookup_board(board_id) {
        return board.thermal_scheme;
    }
    let combined = format!("{model} {family}").to_ascii_lowercase();
    if combined.contains("victus") {
        if combined.contains("16-r") || combined.contains("16-s") {
            ThermalProfileScheme::VictusS
        } else {
            ThermalProfileScheme::Victus
        }
    } else if combined.contains("omen") {
        ThermalProfileScheme::OmenV1
    } else {
        ThermalProfileScheme::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_profile_bytes_match_victus_s_and_legacy_schemes() {
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::VictusS, PerformanceMode::Balanced),
            0x30
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::VictusS, PerformanceMode::Performance),
            0x31
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::VictusS, PerformanceMode::Eco),
            0x50
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::OmenV1, PerformanceMode::Quiet),
            0x50
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::Victus, PerformanceMode::Balanced),
            0x00
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::Victus, PerformanceMode::Performance),
            0x01
        );
        assert_eq!(
            thermal_profile_byte(ThermalProfileScheme::OmenV0, PerformanceMode::Eco),
            0x02
        );
    }

    #[test]
    fn fan_percent_maps_cleanly_to_wmi_level() {
        assert_eq!(fan_percent_to_wmi_level(0, 55), 0);
        assert_eq!(fan_percent_to_wmi_level(100, 55), 55);
        assert_eq!(fan_percent_to_wmi_level(50, 55), 28);
        assert_eq!(fan_percent_to_wmi_level(120, 55), 55);
    }

    #[test]
    fn resolves_8c99_and_fallback_models() {
        assert_eq!(
            resolve_scheme_from_identity("8C99", "Victus 16", "HP VICTUS"),
            ThermalProfileScheme::VictusS
        );
        assert_eq!(
            resolve_scheme_from_identity("ZZZZ", "HP OMEN 16", "HP OMEN"),
            ThermalProfileScheme::OmenV1
        );
    }
}
