//! Native modal dialogs for fatal errors and user confirmations.

use windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO, MessageBoxW,
};
use windows::core::PCWSTR;

/// Display a modal native error dialog for fatal errors.
pub fn show_error_dialog(title: &str, message: &str) {
    let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let msg_wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(msg_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_ICONERROR | MB_OK,
        );
    }
}

/// Display a modal native confirmation (Yes/No) dialog.
pub fn show_confirm_dialog(title: &str, message: &str) -> bool {
    let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let msg_wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let res = MessageBoxW(
            None,
            PCWSTR(msg_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_ICONWARNING | MB_YESNO,
        );
        res == IDYES
    }
}
