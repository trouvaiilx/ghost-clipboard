# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

## Users

Primary users are software engineers, system administrators, and power users working on Windows 11. They frequently copy code snippets, URLs, terminal commands, and text across development environments and productivity tools. Their goal is instant retrieval of clipboard history via keyboard navigation with zero distractions and negligible resource footprint.

## Product Purpose

GhostClipboard provides a lightweight, local-first Windows 11 clipboard history manager. It exists to eliminate clipboard loss without the high resource overhead, cloud telemetry, or distracting web-based interfaces typical of other tools. Success means instant response times (under 50ms window activation), first-class keyboard navigation, and reliable 100% offline persistence.

## Operating Context

Runs continuously as a background process on Windows 11. Activated globally via `Ctrl+Shift+Space` or the system tray. Interfaces directly with the Windows clipboard format listener (`WM_CLIPBOARDUPDATE`), the Win32 active window manager, and the local file system.

## Positioning

Unlike electron-based or cloud-synced clipboard managers that consume hundreds of megabytes of RAM and require cloud accounts, GhostClipboard is a compiled native Rust binary with an embedded SQLite database in WAL mode and software CPU rendering (`tiny-skia`). It uses 25-35 MB RAM idle, requires zero cloud dependencies, transmits no telemetry, and preserves passwords by automatically excluding password managers.

## Capabilities and Constraints

- Event-driven clipboard capture for Unicode text, code snippets, URLs, and multiline text.
- Image and binary file clipboard formats are explicitly out of scope to preserve minimal footprint and high speed.
- Interactive keyboard navigation (arrow keys, Enter to copy and restore focus, Escape to dismiss, Ctrl+P to pin, Delete to remove).
- Process attribution based on Windows foreground window metadata, with automatic exclusion of password manager processes (`KeePass.exe`, `1Password.exe`, `Bitwarden.exe`, etc.).
- Local SQLite persistence in WAL mode with configurable retention limits (entry count and age) and deduplication modes.
- High-density compact UI, combining fast keyboard recall with readable text previews, source attribution tags, and relative timestamps.
- Zero network access: strictly local storage in `%APPDATA%\GhostClipboard`.

## Brand Commitments

GhostClipboard is a minimalist, keyboard-centric native desktop utility. Dark-themed, high-density, no decorative clutter, no marketing distractions, and zero telemetry.

## Evidence on Hand

- Native Rust implementation with Iced 0.14 and tiny-skia CPU rasterizer.
- SQLite WAL persistence in `src/storage/` with automated migration engine (`user_version = 1`).
- Platform integration layer in `src/platform/windows/` (`WM_CLIPBOARDUPDATE`, `RegisterHotKey`, `CreateMutexW`, `QueryFullProcessImageNameW`).
- Comprehensive automated test suite consisting of 53 passing tests covering persistence, UI navigation, Win32 integration, and privacy settings.

## Product Principles

1. Speed and Keyboard Ergonomics: Every core action must be accessible in fewer than three keystrokes without touching the mouse.
2. Local Privacy by Default: Data never leaves the machine; diagnostic logs must never output clipboard contents; sensitive password managers are excluded.
3. Low Overhead: Negligible idle CPU usage through event-driven OS notifications rather than polling; low memory consumption through CPU rasterization and bounded buffers.
4. Predictable Native Utility: The window dismisses cleanly upon copying and restores focus back to the target application immediately.
