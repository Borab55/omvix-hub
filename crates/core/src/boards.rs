use crate::models::{BoardProfile, KeyboardType, ThermalProfileScheme};
use std::borrow::Cow;

/// Known HP Victus & OMEN motherboard IDs aligned with upstream `hp-wmi.c` and community boards.
const KNOWN_BOARDS: &[BoardProfile] = &[
    BoardProfile {
        board_id: Cow::Borrowed("8C99"),
        marketing_name: Cow::Borrowed("HP Victus 16-r1xxx (Intel 14th Gen HX)"),
        thermal_scheme: ThermalProfileScheme::VictusS,
        expected_has_mux: true,
        expected_keyboard: KeyboardType::SingleZoneRgb,
        expected_charge_limit: true,
        notes: Cow::Borrowed("Primary reference test platform (i7-14700HX + RTX 4070)."),
    },
    BoardProfile {
        board_id: Cow::Borrowed("8C9C"),
        marketing_name: Cow::Borrowed("HP Victus 16-s1xxx / OMEN Transcend (AMD)"),
        thermal_scheme: ThermalProfileScheme::VictusS,
        expected_has_mux: true,
        expected_keyboard: KeyboardType::SingleZoneRgb,
        expected_charge_limit: true,
        notes: Cow::Borrowed("Victus-S thermal table family."),
    },
    BoardProfile {
        board_id: Cow::Borrowed("8BBE"),
        marketing_name: Cow::Borrowed("HP Victus 16-r0xxx (Intel 13th Gen)"),
        thermal_scheme: ThermalProfileScheme::VictusS,
        expected_has_mux: true,
        expected_keyboard: KeyboardType::SingleZoneRgb,
        expected_charge_limit: true,
        notes: Cow::Borrowed("2023 Victus 16 Intel series."),
    },
    BoardProfile {
        board_id: Cow::Borrowed("8BD5"),
        marketing_name: Cow::Borrowed("HP Victus 16-s0xxx (AMD Ryzen 7040)"),
        thermal_scheme: ThermalProfileScheme::VictusS,
        expected_has_mux: true,
        expected_keyboard: KeyboardType::SingleZoneRgb,
        expected_charge_limit: true,
        notes: Cow::Borrowed("2023 Victus 16 AMD series."),
    },
    BoardProfile {
        board_id: Cow::Borrowed("8A25"),
        marketing_name: Cow::Borrowed("HP Victus 16-d1xxx / e1xxx"),
        thermal_scheme: ThermalProfileScheme::Victus,
        expected_has_mux: false,
        expected_keyboard: KeyboardType::WhiteBacklight,
        expected_charge_limit: true,
        notes: Cow::Borrowed(
            "2022 Victus 16 series (uses 0x00/0x01/0x02 or Victus thermal profile).",
        ),
    },
    BoardProfile {
        board_id: Cow::Borrowed("8A4F"),
        marketing_name: Cow::Borrowed("HP Victus 15-fa0xxx"),
        thermal_scheme: ThermalProfileScheme::Victus,
        expected_has_mux: false,
        expected_keyboard: KeyboardType::WhiteBacklight,
        expected_charge_limit: true,
        notes: Cow::Borrowed("Victus 15 series."),
    },
    BoardProfile {
        board_id: Cow::Borrowed("8A15"),
        marketing_name: Cow::Borrowed("HP OMEN 16-k0xxx / b1xxx"),
        thermal_scheme: ThermalProfileScheme::OmenV1,
        expected_has_mux: true,
        expected_keyboard: KeyboardType::FourZoneRgb,
        expected_charge_limit: true,
        notes: Cow::Borrowed("OMEN 16 2022 Intel series."),
    },
    BoardProfile {
        board_id: Cow::Borrowed("8BAB"),
        marketing_name: Cow::Borrowed("HP OMEN 16-wf0xxx / xd0xxx"),
        thermal_scheme: ThermalProfileScheme::OmenV1,
        expected_has_mux: true,
        expected_keyboard: KeyboardType::FourZoneRgb,
        expected_charge_limit: true,
        notes: Cow::Borrowed("OMEN 16 2023/2024 series."),
    },
    BoardProfile {
        board_id: Cow::Borrowed("8A16"),
        marketing_name: Cow::Borrowed("HP OMEN 17-ck1xxx"),
        thermal_scheme: ThermalProfileScheme::OmenV1,
        expected_has_mux: true,
        expected_keyboard: KeyboardType::FourZoneRgb,
        expected_charge_limit: true,
        notes: Cow::Borrowed("OMEN 17 series."),
    },
];

pub fn lookup_board(board_id: &str) -> Option<BoardProfile> {
    let normalized = board_id.trim().to_ascii_uppercase();
    KNOWN_BOARDS
        .iter()
        .find(|b| b.board_id.eq_ignore_ascii_case(&normalized))
        .cloned()
}

pub fn all_known_boards() -> &'static [BoardProfile] {
    KNOWN_BOARDS
}

/// Returns `true` when BIOS WMI writes are authorized either because `board_id` is in the
/// verified board table or because the user explicitly approved Experimental Mode for this board.
pub fn is_bios_write_allowed(board_id: &str, experimental_approved_board: Option<&str>) -> bool {
    let trimmed = board_id.trim();
    if trimmed.is_empty() {
        return false;
    }
    if lookup_board(trimmed).is_some() {
        return true;
    }
    experimental_approved_board
        .map(|approved| approved.trim().eq_ignore_ascii_case(trimmed))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_8c99_returns_victus_s_profile() {
        let board = lookup_board("8c99").expect("8C99 must exist in board DB");
        assert_eq!(board.board_id, "8C99");
        assert_eq!(board.thermal_scheme, ThermalProfileScheme::VictusS);
        assert!(board.expected_has_mux);
        assert!(board.expected_charge_limit);
    }

    #[test]
    fn bios_write_gate_blocks_unapproved_unknown_boards() {
        assert!(is_bios_write_allowed("8C99", None));
        assert!(!is_bios_write_allowed("ZZZZ", None));
        assert!(!is_bios_write_allowed("", Some("")));
        assert!(is_bios_write_allowed("ZZZZ", Some("zzzz")));
        assert!(!is_bios_write_allowed("ZZZZ", Some("YYYY")));
    }
}
