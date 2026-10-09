//! Platform integration layer.
//!
//! Everything that touches the operating system lives below this module so the
//! rest of the application can be tested without a desktop session.

#[cfg(windows)]
pub mod windows;

/// A global shortcut: modifier keys plus one Windows virtual-key code.
///
/// Parsing from and persisting to settings is added together with the settings
/// screen; this type is the runtime representation handed to the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    /// Windows virtual-key code (e.g. `0x20` for Space).
    pub vk: u32,
}

impl Hotkey {
    pub const VK_SPACE: u32 = 0x20;

    /// The default shortcut, `Ctrl+Shift+Space`.
    pub const DEFAULT: Hotkey = Hotkey {
        ctrl: true,
        shift: true,
        alt: false,
        win: false,
        vk: Self::VK_SPACE,
    };
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.win {
            f.write_str("Win+")?;
        }
        match self.vk {
            Self::VK_SPACE => f.write_str("Space"),
            // Digits and letters share their ASCII value with the key code.
            0x30..=0x39 | 0x41..=0x5A => write!(f, "{}", char::from(self.vk as u8)),
            0x70..=0x87 => write!(f, "F{}", self.vk - 0x6F),
            other => write!(f, "Key {other:#04x}"),
        }
    }
}

/// Events emitted by the native shell thread towards the application.
///
/// The enum deliberately carries no clipboard content: a change notification is
/// only a signal. Reading the clipboard is a separate, fallible step performed
/// off the UI thread (see the capture service).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    /// `WM_CLIPBOARDUPDATE` was received.
    ClipboardChanged,
    /// The configured global shortcut was pressed.
    HotkeyPressed,
    /// The user clicked the tray icon.
    TrayActivated,
    /// The user chose an entry in the tray menu.
    Menu(TrayCommand),
    /// Events were dropped because the application was not consuming them fast
    /// enough. Carries the number of dropped notifications.
    Overflow(u32),
}

/// Entries of the tray context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    OpenHistory,
    TogglePause,
    OpenSettings,
    Exit,
}

/// Native components managed by the shell thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    ClipboardListener,
    GlobalHotkey,
    TrayIcon,
}

/// A non-fatal start-up problem in one native component.
///
/// The application can still run without any single component (for instance
/// when another program already owns the shortcut), but the user must be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellFault {
    pub component: Component,
    pub message: String,
}

/// Query the currently active foreground window before GhostClipboard takes focus.
pub fn active_window_handle() -> Option<isize> {
    #[cfg(windows)]
    {
        windows::foreground::active_window_handle()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Restore foreground focus to the previously active application window.
pub fn restore_active_window(handle: isize) {
    #[cfg(windows)]
    {
        windows::foreground::restore_active_window(handle);
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
    }
}

/// Write text back to the system clipboard.
pub fn write_clipboard_text(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::clipboard::write_text(text).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkey_is_displayed_in_conventional_order() {
        assert_eq!(Hotkey::DEFAULT.to_string(), "Ctrl+Shift+Space");
    }

    #[test]
    fn letters_digits_and_function_keys_have_readable_names() {
        let key = |vk| Hotkey {
            ctrl: true,
            shift: false,
            alt: true,
            win: false,
            vk,
        };
        assert_eq!(key(u32::from(b'V')).to_string(), "Ctrl+Alt+V");
        assert_eq!(key(u32::from(b'7')).to_string(), "Ctrl+Alt+7");
        assert_eq!(key(0x70).to_string(), "Ctrl+Alt+F1");
        assert_eq!(key(0x7B).to_string(), "Ctrl+Alt+F12");
    }
}
