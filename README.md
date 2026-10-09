# GhostClipboard

[![Platform](https://img.shields.io/badge/platform-Windows%2011%20x64-blue.svg)](https://microsoft.com)
[![Rust](https://img.shields.io/badge/rust-stable%202024%20edition-orange.svg)](https://www.rust-lang.org)
[![GUI](https://img.shields.io/badge/gui-Iced%200.14-cyan.svg)](https://iced.rs)
[![Storage](https://img.shields.io/badge/storage-SQLite%20(WAL)-lightgrey.svg)](https://sqlite.org)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green.svg)](#license)

**GhostClipboard** is a lightweight, keyboard-first, native Windows 11 clipboard history manager written in Rust. It runs quietly in the background, automatically tracks clipboard changes via native Windows event notifications, and lets you instantly search and retrieve previously copied content without ever freezing your workflow or leaking sensitive data.

---

## Key Features

* **Event-Driven Clipboard Monitoring:** Uses Windows native `AddClipboardFormatListener` and `WM_CLIPBOARDUPDATE` messages. Zero continuous polling; negligible CPU and RAM footprint.
* **Keyboard-First Interface:** Built with [Iced 0.14](https://iced.rs) using a CPU renderer (`tiny-skia`). Opens instantly, auto-focuses search, and supports full keyboard navigation (arrows, Enter, Del, Ctrl+P, Esc).
* **Automatic Focus Restoration:** Selecting an entry copies it to the Windows clipboard, dismisses the history window, and restores focus back to your previous application window so you can paste immediately.
* **Strict Local Privacy Guarantee:**
  * 100% offline operation. Zero telemetry, zero analytics, zero cloud synchronization, and no network sockets opened.
  * Sensitive clipboard contents, URLs, and code snippets are **never** logged or emitted in tracing outputs.
  * Local storage in `%APPDATA%\GhostClipboard\history.db` using SQLite with Write-Ahead Logging (WAL) and `%APPDATA%\GhostClipboard\settings.json`.
* **Password Manager & Application Exclusions:** Automatically skips clipboard copies originating from excluded process executables (`1Password.exe`, `KeePass.exe`, `Bitwarden.exe`, `KeePassXC.exe`, `Enpass.exe`, and custom entries).
* **Configurable Retention & Deduplication:**
  * Deduplication options: Move existing entry to top or keep consecutive duplicates only.
  * Retention limits: Bounded max unpinned entries (e.g. 200, 500, 1,000, 5,000) and max age (e.g. 7 days, 30 days, 90 days, 1 year).
  * Pinned entries are protected from retention sweeps.
  * Maximum clipboard entry size limit (e.g. 512 KB, 1 MB, 2 MB, 5 MB, 10 MB).
* **System Tray & Global Hotkey:**
  * Global Hotkey: `Ctrl+Shift+Space` toggles the history window anywhere in Windows.
  * Tray icon with context menu: Open History, Pause/Resume Recording, Settings, and Exit.
  * Single-instance safeguard via named Windows Mutex (`Local\GhostClipboard_SingleInstance_Mutex`). Launching a second instance activates the running window and exits cleanly.
* **Launch at Login:** Optional Windows Registry startup integration (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`) with `--background` flag support.

---

## Architecture Overview

GhostClipboard uses a strictly isolated multi-threaded architecture that keeps OS-specific event loops, disk I/O, and GUI rendering completely separated:

```
┌────────────────────────────────────────────────────────────────────────┐
│                        GhostClipboard Runtime                          │
├──────────────────────┬─────────────────────────────────────────────────┤
│  Win32 Shell Thread  │ • Hidden Message-Only HWND                      │
│                      │ • AddClipboardFormatListener / WM_CLIPBOARDUPDATE│
│                      │ • RegisterHotKey (Ctrl+Shift+Space)             │
│                      │ • tray-icon Notification Loop                   │
│                      │ • Single-Instance Mutex & HWND Activation       │
├──────────────────────┼─────────────────────────────────────────────────┤
│   Tokio Runtime      │ • Channel coordinator                           │
│   (Orchestrator)     │ • Capture pipeline: validation & dedup filter   │
│                      │ • Process attribution via foreground HWND       │
│                      │ • Settings persistence (JSON)                   │
├──────────────────────┼─────────────────────────────────────────────────┤
│  Storage Thread      │ • Dedicated OS thread for SQLite                │
│                      │ • rusqlite in WAL mode with user_version schema │
│                      │ • Transactional indexing, retention, & FTS-ready│
├──────────────────────┼─────────────────────────────────────────────────┤
│   Iced GUI Thread    │ • Iced 0.14 application loop                    │
│                      │ • Dark, high-density split-pane interface       │
│                      │ • CPU tiny-skia renderer (zero GPU driver faults)│
│                      │ • Window focus & previous HWND restoration      │
└──────────────────────┴─────────────────────────────────────────────────┘
```

---

## Keyboard Shortcuts

| Shortcut | Description |
| :--- | :--- |
| `Ctrl + Shift + Space` | Global hotkey to open or dismiss GhostClipboard from any application. |
| `Arrow Up` / `Arrow Down` | Navigate previous / next clipboard entry in the search results. |
| `Enter` | Copy selected entry to the Windows clipboard, dismiss window, and focus previous app. |
| `Escape` | Dismiss history window; if search query is active, clears search first. |
| `Ctrl + P` | Pin / unpin selected entry (pinned entries survive retention sweeps). |
| `Delete` or `Ctrl + D` | Permanently delete the selected clipboard entry. |

---

## Privacy & Security Model

1. **Local-Only Storage:** All clipboard items are stored exclusively in SQLite at `%APPDATA%\GhostClipboard\history.db`. Settings are stored at `%APPDATA%\GhostClipboard\settings.json`.
2. **Zero Telemetry:** No HTTP requests, telemetry packets, analytics, or background update pings are ever executed.
3. **Log Sanitization:** Tracing logs strictly record metadata (character lengths, deduplication outcomes, sequence IDs) and **never** the text contents of the clipboard, preventing leaks into log files.
4. **Foreground Process Exclusion:** When a clipboard change occurs, GhostClipboard inspects the active Windows foreground window (`GetForegroundWindow` -> `GetWindowThreadProcessId` -> `QueryFullProcessImageNameW`). If the process matches an excluded application (e.g. password manager), the clipboard notification is dropped immediately.
   > **Note on Attribution Timing:** Process attribution queries the foreground process at notification time. Avoid instantaneous rapid window-switching while copying highly sensitive credentials.

---

## Building & Running

### Prerequisites

* Windows 11 (64-bit).
* Rust toolchain (stable 1.80+):
  ```powershell
  rustup default stable-x86_64-pc-windows-msvc
  ```

### Development Build & Test

```powershell
# Clone the repository
git clone https://github.com/Yuuji/ghost-clipboard.git
cd ghost-clipboard

# Run comprehensive test suite (53 tests)
cargo test

# Run quality checks
cargo fmt --check
cargo clippy --all-targets -- -D warnings

# Run in development mode
cargo run
```

### Release Build

```powershell
# Compile optimized release binary
cargo build --release

# The compiled binary is located at:
# target\release\ghostclipboard.exe
```

### Command Line Flags

* `ghostclipboard.exe` - Starts GhostClipboard and displays the history interface.
* `ghostclipboard.exe --background` - Starts GhostClipboard minimized to the Windows system tray (used by Windows autostart at login).

---

## Repository Structure

```
ghost-clipboard/
├── Cargo.toml                  # Dependencies & build configuration
├── src/
│   ├── main.rs                 # CLI entry point, single-instance check, Iced daemon launch
│   ├── lib.rs                  # Library crate root
│   ├── branding.rs             # Generated window icon and tray icon rendering
│   ├── app/                    # Iced application state, messages, theme, and views
│   │   ├── mod.rs              # State management & update message dispatch
│   │   ├── theme.rs            # Clean dark utility theme palette & styles
│   │   ├── time.rs             # Relative timestamp formatting helper
│   │   └── view.rs             # Modular UI view rendering (split-pane & settings cards)
│   ├── domain/                 # Platform-agnostic domain models & policies
│   │   ├── mod.rs
│   │   ├── capture.rs          # Size limits, UTF-8 normalization, validation
│   │   ├── entry.rs            # Entry models, ContentHash, ContentKind
│   │   └── settings.rs         # AppSettings, DedupMode, RetentionPolicy, JSON persistence
│   ├── platform/               # Operating system abstraction layer
│   │   ├── mod.rs
│   │   └── windows/            # Native Win32 platform implementation
│   │       ├── autostart.rs    # Registry HKCU\Run launch-at-login
│   │       ├── clipboard.rs    # Win32 OpenClipboard/GetClipboardData/SetClipboardData
│   │       ├── foreground.rs   # Foreground HWND process name detection
│   │       ├── shell.rs        # Win32 hidden message loop, hotkey, and tray integration
│   │       └── single_instance.rs # Named Windows Mutex guard
│   ├── services/               # Background business services
│   │   ├── mod.rs
│   │   └── capture.rs          # Event-driven clipboard capture orchestrator
│   └── storage/                # SQLite persistence layer
│       ├── mod.rs
│       ├── migrations.rs       # SQLite schema migration engine (user_version)
│       ├── repository.rs       # Synchronous SQLite repository & queries
│       └── worker.rs           # Dedicated storage background thread & actor handle
├── tests/                      # Integration test suites
│   ├── clipboard_test.rs       # Win32 clipboard read/write roundtrip tests
│   ├── history_ui_test.rs      # UI state, keyboard navigation, and filtering tests
│   ├── persistence_test.rs     # SQLite WAL persistence and retention tests
│   ├── settings_test.rs        # Settings JSON persistence and exclusion tests
│   └── system_integration_test.rs # Single-instance and registry autostart tests
└── docs/
    └── ARCHITECTURE.md         # Detailed architectural documentation
```

---

## License

Dual-licensed under either of:

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
