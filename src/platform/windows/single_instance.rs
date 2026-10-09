//! Windows single-instance guard using a named Mutex.
//!
//! Ensures only one copy of GhostClipboard runs at a time. If a secondary instance
//! is launched, it finds the existing running instance, sends an activation signal
//! to bring the history window to the foreground, and exits cleanly.

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, HWND_MESSAGE, PostMessageW, WM_APP};
use windows::core::w;

pub const WM_ACTIVATE_INSTANCE: u32 = WM_APP + 2;

/// RAII guard holding the single-instance mutex.
/// Releasing or dropping this struct releases the mutex.
pub struct SingleInstance {
    handle: HANDLE,
}

// SAFETY: HANDLE is safe to send between threads when only closed upon drop.
unsafe impl Send for SingleInstance {}
unsafe impl Sync for SingleInstance {}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        if !self.handle.is_invalid() && !self.handle.0.is_null() {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
            tracing::debug!("single-instance mutex released");
        }
    }
}

impl SingleInstance {
    /// Attempt to acquire the single-instance guard.
    ///
    /// * Returns `Ok(Some(guard))` if this is the first running instance.
    /// * Returns `Ok(None)` if another instance is already active (and signals it to open).
    /// * Returns `Err(...)` on operating system errors.
    pub fn acquire() -> Result<Option<Self>, windows::core::Error> {
        Self::acquire_with_options(w!("Local\\GhostClipboard_SingleInstance_Mutex"), true)
    }

    /// Internal acquisition allowing custom mutex name and signal toggling (for tests).
    pub fn acquire_with_options(
        name: windows::core::PCWSTR,
        signal_existing: bool,
    ) -> Result<Option<Self>, windows::core::Error> {
        let handle = unsafe { CreateMutexW(None, true, name) }?;

        let last_error = unsafe { GetLastError() };
        if last_error == ERROR_ALREADY_EXISTS {
            tracing::info!(
                "another GhostClipboard instance is already running; activating existing window"
            );

            if signal_existing {
                // Attempt to locate the existing background shell window and signal it to show
                unsafe {
                    let existing_hwnd: Result<HWND, _> =
                        FindWindowExW(Some(HWND_MESSAGE), None, w!("GhostClipboardShell"), None);
                    if let Ok(hwnd) = existing_hwnd
                        && !hwnd.is_invalid()
                        && !hwnd.0.is_null()
                    {
                        let _ =
                            PostMessageW(Some(hwnd), WM_ACTIVATE_INSTANCE, WPARAM(0), LPARAM(0));
                    }
                }
            }

            unsafe {
                let _ = CloseHandle(handle);
            }

            Ok(None)
        } else {
            tracing::debug!("acquired single-instance mutex");
            Ok(Some(Self { handle }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_instance_acquire_and_release() {
        let test_mutex = w!("Local\\GhostClipboard_Test_SingleInstance_Mutex");
        let first = SingleInstance::acquire_with_options(test_mutex, false)
            .expect("acquire first instance");
        assert!(first.is_some(), "first instance must succeed");

        // Attempting to acquire again while `first` is alive should report already exists
        let second = SingleInstance::acquire_with_options(test_mutex, false)
            .expect("acquire second instance check");
        assert!(
            second.is_none(),
            "second instance must detect already running"
        );

        // Dropping first releases mutex
        drop(first);

        // Third acquisition now succeeds
        let third =
            SingleInstance::acquire_with_options(test_mutex, false).expect("acquire after drop");
        assert!(third.is_some(), "re-acquisition after drop must succeed");
    }
}
