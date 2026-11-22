use std::collections::{BTreeSet, HashSet};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

const NOISY_SYSTEM_EXES: &[&str] = &[
    "svchost.exe",
    "csrss.exe",
    "conhost.exe",
    "dwm.exe",
    "smss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "fontdrvhost.exe",
    "runtimebroker.exe",
    "sihost.exe",
    "taskhostw.exe",
    "ctfmon.exe",
    "dllhost.exe",
    "searchindexer.exe",
    "audiodg.exe",
    "spoolsv.exe",
    "wmiprvse.exe",
    "omvix-hub.exe",
    "omvix-cli.exe",
    "system",
    "registry",
    "memory compression",
    "idle",
];

pub fn extract_wide_nul_terminated(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn is_noisy_system_exe(lower_name: &str) -> bool {
    let trimmed = lower_name.trim();
    NOISY_SYSTEM_EXES
        .iter()
        .any(|noisy| trimmed.eq_ignore_ascii_case(noisy))
}

pub fn running_process_names_lower() -> HashSet<String> {
    let mut set = HashSet::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return set;
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let name = extract_wide_nul_terminated(&entry.szExeFile)
                .trim()
                .to_ascii_lowercase();
            if !name.is_empty() {
                set.insert(name);
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    unsafe {
        let _ = CloseHandle(snapshot);
    }
    set
}

pub fn list_user_running_exes() -> Vec<String> {
    let mut filtered = BTreeSet::new();
    for name in running_process_names_lower() {
        if name.ends_with(".exe") && !is_noisy_system_exe(&name) {
            filtered.insert(name);
        }
    }
    filtered.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_wide_nul_terminated_stops_at_first_nul() {
        let mut buf = [0u16; 32];
        for (i, ch) in "chrome.exe".encode_utf16().enumerate() {
            buf[i] = ch;
        }
        buf[15] = b'x' as u16;
        assert_eq!(extract_wide_nul_terminated(&buf), "chrome.exe");
    }

    #[test]
    fn noisy_system_exes_are_filtered_out() {
        assert!(is_noisy_system_exe("svchost.exe"));
        assert!(is_noisy_system_exe("OMVIX-HUB.EXE"));
        assert!(is_noisy_system_exe("System"));
        assert!(!is_noisy_system_exe("cyberpunk2077.exe"));
        assert!(!is_noisy_system_exe("blender.exe"));
    }
}
