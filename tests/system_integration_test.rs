//! Integration tests for Windows single-instance safeguard, autostart, and shell activation.

use ghost_clipboard::platform::windows::shell::WM_ACTIVATE_INSTANCE;
use ghost_clipboard::platform::windows::single_instance::SingleInstance;
use ghost_clipboard::platform::windows::{Shell, ShellConfig};
use ghost_clipboard::platform::{Hotkey, ShellEvent, is_autostart_enabled};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::w;

#[test]
fn single_instance_guard_behavior() {
    let test_mutex = w!("Local\\GhostClipboard_SystemIntegration_Mutex");

    // Acquire first
    let guard1 =
        SingleInstance::acquire_with_options(test_mutex, false).expect("acquire first test mutex");
    assert!(guard1.is_some(), "first acquire should succeed");

    // Acquire second while first is held
    let guard2 =
        SingleInstance::acquire_with_options(test_mutex, false).expect("acquire second test mutex");
    assert!(guard2.is_none(), "second acquire should detect conflict");

    // Drop first
    drop(guard1);

    // Acquire again after drop
    let guard3 =
        SingleInstance::acquire_with_options(test_mutex, false).expect("acquire third test mutex");
    assert!(guard3.is_some(), "third acquire should succeed after drop");
}

#[test]
fn autostart_query_succeeds() {
    let status = is_autostart_enabled();
    assert!(status.is_ok(), "querying autostart status should not fail");
}

#[test]
fn shell_activates_on_instance_signal() {
    let config = ShellConfig {
        hotkey: Hotkey {
            ctrl: true,
            shift: true,
            alt: true,
            win: false,
            vk: 0x86, // F23
        },
    };
    let mut shell = Shell::spawn(config).expect("start test shell");
    let mut events = shell.take_events().expect("take shell events");

    // Post WM_ACTIVATE_INSTANCE to the shell window handle
    let hwnd = HWND(shell.hwnd() as *mut _);
    unsafe {
        let res = PostMessageW(Some(hwnd), WM_ACTIVATE_INSTANCE, WPARAM(0), LPARAM(0));
        assert!(res.is_ok(), "posting WM_ACTIVATE_INSTANCE must succeed");
    }

    // Give message loop a moment to dispatch
    let event = events.blocking_recv();
    assert_eq!(
        event,
        Some(ShellEvent::HotkeyPressed),
        "WM_ACTIVATE_INSTANCE must emit HotkeyPressed to bring window up"
    );
}
