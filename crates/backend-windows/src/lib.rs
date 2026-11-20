//! Windows backend crate for Omvix Hub (COM/WMI `hpqBIntM`, Win32, NVML).
pub fn backend_ready() -> bool {
    cfg!(target_os = "windows")
}
