//! Foreground window inspection.
//!
//! Retrieves the executable name of the currently active window
//! to attribute clipboard entries to source applications, and
//! saves/restores active window focus across history display.

use std::path::Path;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId, IsWindow,
    SetForegroundWindow,
};
use windows::core::PWSTR;

static LAST_EXTERNAL_FOREGROUND: AtomicIsize = AtomicIsize::new(0);

/// Check whether `hwnd` belongs to the Windows shell or taskbar.
pub fn is_shell_or_taskbar(hwnd: HWND) -> bool {
    let mut class_name = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut class_name) };
    if len > 0 {
        let name = String::from_utf16_lossy(&class_name[..len as usize]);
        matches!(
            name.as_str(),
            "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "NotifyIconOverflowWindow"
                | "Progman"
                | "WorkerW"
        )
    } else {
        false
    }
}

/// Update the tracked last external foreground window.
pub fn update_last_external_foreground(hwnd: HWND) {
    if hwnd.is_invalid() || hwnd.0.is_null() {
        return;
    }
    let mut pid: u32 = 0;
    let _ = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 || pid == std::process::id() {
        return;
    }
    if !is_shell_or_taskbar(hwnd) && unsafe { IsWindow(Some(hwnd)).as_bool() } {
        LAST_EXTERNAL_FOREGROUND.store(hwnd.0 as isize, Ordering::SeqCst);
    }
}

/// Get the handle of the currently active foreground window as an isize integer.
/// Returns None if the foreground window is invalid or belongs to our process.
/// If current foreground is taskbar or invalid, falls back to the last tracked external foreground window.
pub fn active_window_handle() -> Option<isize> {
    unsafe {
        let fg: HWND = GetForegroundWindow();
        if !fg.is_invalid() && !fg.0.is_null() {
            let mut pid: u32 = 0;
            let _ = GetWindowThreadProcessId(fg, Some(&mut pid));
            if pid != 0
                && pid != std::process::id()
                && !is_shell_or_taskbar(fg)
                && IsWindow(Some(fg)).as_bool()
            {
                LAST_EXTERNAL_FOREGROUND.store(fg.0 as isize, Ordering::SeqCst);
                return Some(fg.0 as isize);
            }
        }

        // Fall back to tracked last external foreground window
        let last = LAST_EXTERNAL_FOREGROUND.load(Ordering::SeqCst);
        if last != 0 {
            let hwnd = HWND(last as *mut _);
            if IsWindow(Some(hwnd)).as_bool() {
                return Some(last);
            }
        }

        None
    }
}

/// Restore focus to a previously active window.
///
/// Validates with `IsWindow`, checks `SetForegroundWindow`'s return value,
/// and applies thread input attachment as a fallback.
pub fn restore_active_window(handle: isize) -> bool {
    if handle == 0 {
        return false;
    }
    unsafe {
        let hwnd = HWND(handle as *mut _);
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }

        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }

        // Fallback: attach thread input to bypass Windows focus restrictions
        let current_thread = GetCurrentThreadId();
        let target_thread = GetWindowThreadProcessId(hwnd, None);

        if current_thread != target_thread && target_thread != 0 {
            let _ = AttachThreadInput(current_thread, target_thread, true);
            let success = SetForegroundWindow(hwnd).as_bool();
            let _ = BringWindowToTop(hwnd);
            let _ = AttachThreadInput(current_thread, target_thread, false);
            if success {
                return true;
            }
        }

        BringWindowToTop(hwnd).is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restore_active_window_null_or_invalid() {
        assert!(!restore_active_window(0));
        assert!(!restore_active_window(-1));
        assert!(!restore_active_window(999_999_999));
    }

    #[test]
    fn test_is_shell_or_taskbar_null() {
        assert!(!is_shell_or_taskbar(HWND(std::ptr::null_mut())));
    }

    #[test]
    fn test_update_last_external_foreground_null() {
        update_last_external_foreground(HWND(std::ptr::null_mut()));
        // Null HWND should not panic and should not set an invalid pointer
    }
}
