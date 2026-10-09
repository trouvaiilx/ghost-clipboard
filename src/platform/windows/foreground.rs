//! Foreground window inspection.
//!
//! Retrieves the executable name of the currently active window
//! to attribute clipboard entries to source applications.

use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

/// Get the executable name (e.g. `code.exe`, `notepad.exe`) of the foreground window.
/// Returns `None` if the foreground window cannot be queried or belongs to GhostClipboard.
pub fn foreground_process_name() -> Option<String> {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.is_invalid() || hwnd.0.is_null() {
            return None;
        }

        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return None;
        }

        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        if handle.is_invalid() {
            return None;
        }

        let mut buffer = [0u16; 1024];
        let mut len: u32 = buffer.len() as u32;

        let res = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );

        let _ = CloseHandle(handle);

        if res.is_ok() && len > 0 {
            let path_str = String::from_utf16_lossy(&buffer[..len as usize]);
            let path = Path::new(&path_str);
            let exe_name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or(path_str);
            Some(exe_name)
        } else {
            None
        }
    }
}
