# Validation log

This file records what has actually been verified, how, and what has not. It is updated at each
phase. "Verified" means observed running on Windows, not merely compiled.

## Environment

- Windows 11 x64, Rust 1.98.1 (`stable-x86_64-pc-windows-msvc`).
- **Limitation:** the development sandbox is a non-interactive desktop session. There is no
  taskbar (`Shell_TrayWnd` absent), no foreground window, the cursor is fixed at (0,0), and synthetic
  keyboard input (`keybd_event`) is not visible to `GetAsyncKeyState`. Anything requiring real user
  input or the visible notification area is therefore **not verified here**.

## Phase 1

| Item | How | Result |
| --- | --- | --- |
| Project builds (`cargo build`) | dev profile | Pass |
| Clipboard notifications arrive from `AddClipboardFormatListener` | Running binary + PowerShell `Set-Clipboard` twice, observing debug log | Pass: events received (3 per copy; see ARCHITECTURE.md) |
| Shell starts with no component faults | Unit test + log inspection | Pass |
| `RegisterHotKey` succeeds, conflicts are detected and explained, shutdown releases the shortcut | Unit tests with real registration | Pass |
| `WM_HOTKEY` → `ShellEvent::HotkeyPressed`; unknown hotkey ids ignored | Unit test (synthetic message to shell window) | Pass |
| Event channel is bounded; overflow is reported | Unit test (100 notifications, 64-slot queue) | Pass |
| Iced daemon starts with no window in `--background` mode | Window enumeration of the running process | Pass |
| Window opens, close request hides to tray (process stays alive), and show/hide toggles repeatedly | Window enumeration while a temporary timer emitted `HotkeyPressed` (timer code not committed) | Pass |
| Idle footprint (debug build) | `Get-Process`: ~13.6 MB working set with no window, ~37.7 MB with window shown; 0.1–0.2 s total CPU after several seconds | Informational only; debug build, release not measured yet |
| Tray icon visible in notification area | - | **Not verified** (no taskbar in sandbox). `tray-icon` creation reports success. |
| Tray left-click and menu selection | Attempted by posting tray-icon's internal callback message | **Not verified**: the library drops clicks when the icon has no shell rect, as is the case in this sandbox |
| Global shortcut pressed on a real keyboard | Attempted with `keybd_event` | **Not verified** (synthetic input is not processed in this session). Needs manual check: run the app, press `Ctrl+Shift+Space`. |

### Manual checks to run on an interactive desktop

1. `cargo run`; confirm the ghost icon appears in the notification area (it may be under the chevron).
2. Press `Ctrl+Shift+Space` from another application; the window should toggle.
3. Left-click the tray icon (open) and right-click it (menu: Open history, Pause capture, Settings, Exit).
4. Choose **Exit** from the tray menu. Note: Phase 1 only wires *Open history* and *Exit*.

