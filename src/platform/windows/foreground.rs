//! Foreground window inspection.
//!
//! Retrieves the executable name of the currently active window
//! to attribute clipboard entries to source applications, and
//! saves/restores active window focus across history display.

use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
};
use windows::core::PWSTR;

/// Get the handle of the currently active foreground window as an isize integer.
/// Returns None if the foreground window is invalid or belongs to our process.
pub fn active_window_handle() -> Option<isize> {
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

        Some(hwnd.0 as isize)
    }
}

/// Restore focus to a previously active window.
pub fn restore_active_window(handle: isize) {
    if handle != 0 {
        unsafe {
            let hwnd = HWND(handle as *mut _);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

fn process_name_for_hwnd(hwnd: HWND) -> Option<String> {
    if hwnd.is_invalid() || hwnd.0.is_null() {
        return None;
    }

    let mut pid: u32 = 0;
    let _ = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 || pid == std::process::id() {
        return None;
    }

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    if handle.is_invalid() {
        return None;
    }

    let mut buffer = [0u16; 1024];
    let mut len: u32 = buffer.len() as u32;

    let res = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    };

    let _ = unsafe { CloseHandle(handle) };

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

/// Get the executable name (e.g. `code.exe`, `notepad.exe`) of the foreground window.
/// Returns `None` if the foreground window cannot be queried or belongs to GhostClipboard.
pub fn foreground_process_name() -> Option<String> {
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        process_name_for_hwnd(hwnd)
    }
}

/// Get the executable name of the clipboard source process.
///
/// Queries `GetClipboardOwner` first to avoid race conditions with fast window switching,
/// and falls back to `GetForegroundWindow` if clipboard owner is not available.
/// Returns `None` if the source cannot be queried or belongs to GhostClipboard.
pub fn source_process_name() -> Option<String> {
    unsafe {
        if let Ok(owner) = windows::Win32::System::DataExchange::GetClipboardOwner()
            && !owner.is_invalid()
            && !owner.0.is_null()
        {
            let mut pid: u32 = 0;
            let _ = GetWindowThreadProcessId(owner, Some(&mut pid));
            if pid == std::process::id() {
                // Clipboard is owned by GhostClipboard itself; do not fall back to foreground
                return None;
            }
            if let Some(name) = process_name_for_hwnd(owner) {
                return Some(name);
            }
        }

        let fg: HWND = GetForegroundWindow();
        process_name_for_hwnd(fg)
    }
}
