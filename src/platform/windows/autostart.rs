//! Windows launch-at-login integration via HKCU Run registry key.

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, REG_VALUE_TYPE, RegCloseKey,
    RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::w;

const RUN_KEY_PATH: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APP_VALUE_NAME: windows::core::PCWSTR = w!("GhostClipboard");

/// Check whether GhostClipboard is configured to start on user login.
pub fn is_autostart_enabled() -> Result<bool, String> {
    unsafe {
        let mut hkey = HKEY::default();
        let res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY_PATH,
            Some(0),
            KEY_QUERY_VALUE,
            &mut hkey,
        );

        if res != ERROR_SUCCESS {
            return Ok(false);
        }

        let mut data_type = REG_VALUE_TYPE::default();
        let mut data_len = 0u32;
        let query_res = RegQueryValueExW(
            hkey,
            APP_VALUE_NAME,
            None,
            Some(&mut data_type),
            None,
            Some(&mut data_len),
        );

        let _ = RegCloseKey(hkey);

        if query_res == ERROR_SUCCESS {
            Ok(true)
        } else if query_res == ERROR_FILE_NOT_FOUND {
            Ok(false)
        } else {
            Err(format!(
                "failed to query registry Run key: error code {query_res:?}"
            ))
        }
    }
}

/// Enable or disable GhostClipboard launch-at-login.
///
/// When enabled, sets `{current_exe} --background` in the Run key.
pub fn set_autostart_enabled(enabled: bool) -> Result<(), String> {
    unsafe {
        let mut hkey = HKEY::default();
        let res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY_PATH,
            Some(0),
            KEY_SET_VALUE,
            &mut hkey,
        );

        if res != ERROR_SUCCESS {
            return Err(format!(
                "failed to open registry Run key for writing: error code {res:?}"
            ));
        }

        if enabled {
            let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let exe_str = current_exe.to_string_lossy().to_string();
            let command = format!("\"{exe_str}\" --background");

            let u16_val: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes_len = u16_val.len() * std::mem::size_of::<u16>();
            let slice = std::slice::from_raw_parts(u16_val.as_ptr() as *const u8, bytes_len);

            let set_res = RegSetValueExW(hkey, APP_VALUE_NAME, Some(0), REG_SZ, Some(slice));

            let _ = RegCloseKey(hkey);

            if set_res == ERROR_SUCCESS {
                tracing::info!(%command, "enabled launch-at-login in Windows registry");
                Ok(())
            } else {
                Err(format!(
                    "failed to set registry value: error code {set_res:?}"
                ))
            }
        } else {
            let del_res = RegDeleteValueW(hkey, APP_VALUE_NAME);
            let _ = RegCloseKey(hkey);

            if del_res == ERROR_SUCCESS || del_res == ERROR_FILE_NOT_FOUND {
                tracing::info!("disabled launch-at-login in Windows registry");
                Ok(())
            } else {
                Err(format!(
                    "failed to delete registry value: error code {del_res:?}"
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_autostart_does_not_crash() {
        // Querying current status should succeed without errors even in CI / sandbox
        let result = is_autostart_enabled();
        assert!(result.is_ok(), "querying autostart status should succeed");
    }
}
