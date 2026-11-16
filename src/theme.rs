use windows::core::PCWSTR;
use windows::Win32::System::Registry::*;

use crate::native_interop::wide_str;

const REGISTRY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const REGISTRY_KEY: &str = "SystemUsesLightTheme";

/// Check if the system is in dark mode by reading the registry
pub fn is_dark_mode() -> bool {
    !is_light_theme()
}

/// Cached dark-mode flag for per-frame render code (F4). The widget refreshes
/// it on `WM_SETTINGCHANGE`; the entry also expires after one second so a
/// process without that hook (theme studio) still follows the system.
pub fn is_dark_mode_cached() -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(bool, Instant)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((dark, at)) = *cache {
        if at.elapsed() < Duration::from_secs(1) && !DARK_DIRTY.load(Ordering::Acquire) {
            return dark;
        }
    }
    DARK_DIRTY.store(false, Ordering::Release);
    let dark = is_dark_mode();
    *cache = Some((dark, Instant::now()));
    dark
}

use std::sync::atomic::{AtomicBool, Ordering};
static DARK_DIRTY: AtomicBool = AtomicBool::new(false);

/// Force the next `is_dark_mode_cached` call to read the registry.
pub fn invalidate_dark_mode_cache() {
    DARK_DIRTY.store(true, Ordering::Release);
}

fn is_light_theme() -> bool {
    unsafe {
        let path = wide_str(REGISTRY_PATH);
        let key_name = wide_str(REGISTRY_KEY);

        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        );

        if result.is_err() {
            return false; // Default to dark mode
        }

        let mut data: u32 = 0;
        let mut data_size: u32 = std::mem::size_of::<u32>() as u32;
        let result = RegQueryValueExW(
            hkey,
            PCWSTR::from_raw(key_name.as_ptr()),
            None,
            None,
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut data_size),
        );

        let _ = RegCloseKey(hkey);

        if result.is_err() {
            return false; // Default to dark mode
        }

        data == 1
    }
}
