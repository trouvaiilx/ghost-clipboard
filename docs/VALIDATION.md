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
| Idle footprint (release build) | `Get-Process`: ~17.8 MB working set (~3.8 MB private bytes) with no window (`--background`); ~35.1 MB working set (~11.2 MB private bytes) with history window shown | Pass: validates the 25–35 MB idle claim in production release profile |
| Tray icon visible in notification area | Verified in tray-icon creation; see manual checklist below | Needs manual desktop check |
| Tray left-click and menu selection | Handled via ShellEvent; see manual checklist below | Needs manual desktop check |
| Global shortcut pressed on a real keyboard | Verified via Win32 RegisterHotKey unit tests; see manual checklist below | Needs manual desktop check |

## Phase 2
| Item | How | Result |
| --- | --- | --- |
| SQLite repository CRUD & migrations | Integration tests in `tests/persistence_test.rs` | Pass: v1 schema created, tables & indexes migrated |
| Durable storage across application restarts | Reopening SQLite database file on disk across separate instances | Pass: pinned status, timestamps, and full contents survive restart |
| Verbatim text preservation | Verifying byte-exact retrieval of multiline, CRLF, leading/trailing whitespace, UTF-8/emojis, code | Pass: stored text is identical to copied text |
| Deduplication rules (`Anywhere` vs `LatestOnly`) | Tests with repeated text copies | Pass: `Anywhere` moves matching entry to top without deleting pinned state; `LatestOnly` adds duplicate only if intervening copy occurred |
| Retention policies (max count and age cutoff) | Tests applying retention with pinned and unpinned entries | Pass: unpinned entries cleaned up according to limits; pinned entries are never deleted |
| Non-blocking storage worker | `Storage` background thread with bounded queue | Pass: CRUD executed via async channel without blocking callers |
| Windows clipboard read/write roundtrip | `tests/clipboard_test.rs` reading and writing `CF_UNICODETEXT` with sequence number check | Pass |
| End-to-end event-driven capture in running app | Running background application process and invoking `Set-Clipboard` | Pass: `WM_CLIPBOARDUPDATE` received, sequence checked, entry persisted to `%APPDATA%/GhostClipboard/history.db` |
| No sensitive clipboard content in logs | Automated test in `tests/privacy_logging_test.rs` + diagnostic audit | Pass: only sequence numbers, lengths, event types, and outcomes are logged |

## Desktop Session Manual Verification Checklist

Because CI and headless sandboxes lack a visible taskbar, notification area (`Shell_TrayWnd`), and interactive keyboard focus, the following items must be verified during an interactive Windows desktop session:

1. **System Tray Icon**:
   - [ ] Launch `ghostclipboard.exe --background`.
   - [ ] Verify the ghost icon appears in the Windows 11 notification area (or taskbar overflow chevron).
   - [ ] Hover cursor: verify tooltip displays `GhostClipboard`.

2. **Tray Context Menu**:
   - [ ] Right-click the tray icon: menu opens with `Open History`, `Pause capture`, `Settings`, `Exit`.
   - [ ] Click `Pause capture`:
     - Tray icon changes to distinct amber paused icon (`branding::render_paused`).
     - Tooltip updates to `GhostClipboard (Paused)`.
     - Menu item updates to `Resume capture`.
     - Copying text to the clipboard while paused does not record entries.
   - [ ] Click `Resume capture`: icon, tooltip, and label restore to normal; captures resume.
   - [ ] Click `Exit`: application shuts down cleanly, releasing hotkey and saving WAL.

3. **Global Shortcut & Window Toggle**:
   - [ ] Press `Ctrl+Shift+Space`: GhostClipboard history window appears centered and gains input focus.
   - [ ] Press `Ctrl+Shift+Space` again (or `Esc`): window closes back to tray.
   - [ ] Verify search input is immediately focused when window opens.

4. **Focus Restoration**:
   - [ ] Focus Notepad or another editor, then press `Ctrl+Shift+Space`.
   - [ ] Select an entry and press `Enter` (or click Copy).
   - [ ] Verify GhostClipboard closes, focus immediately returns to Notepad, and pressing `Ctrl+V` pastes the selected text.

5. **Exclusions**:
   - [ ] Copy text from KeePass, Bitwarden, or 1Password.
   - [ ] Open GhostClipboard: verify sensitive copied text was ignored and never saved to history.


