//! Safe Windows clipboard access.
//!
//! Provides bounded retries for reading and writing text, automatic RAII
//! clipboard release, format validation, and sequence number tracking.

use std::sync::OnceLock;
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::core::w;

pub const CF_UNICODETEXT: u32 = 13;

#[derive(Debug, Clone, Copy)]
pub struct OptOutFormats {
    pub viewer_ignore: u32,
    pub exclude_monitor: u32,
    pub can_include_history: u32,
    pub can_upload_cloud: u32,
}

static OPT_OUT_FORMATS: OnceLock<OptOutFormats> = OnceLock::new();

/// Cached registered clipboard format IDs for privacy and opt-out handling.
pub fn opt_out_formats() -> &'static OptOutFormats {
    OPT_OUT_FORMATS.get_or_init(|| unsafe {
        OptOutFormats {
            viewer_ignore: RegisterClipboardFormatW(w!("Clipboard Viewer Ignore")),
            exclude_monitor: RegisterClipboardFormatW(w!(
                "ExcludeClipboardContentFromMonitorProcessing"
            )),
            can_include_history: RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
            can_upload_cloud: RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")),
        }
    })
}

unsafe fn read_clipboard_dword(format: u32) -> Option<u32> {
    if format == 0 {
        return None;
    }
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        if handle.is_invalid() || handle.0.is_null() {
            return None;
        }
        let hglobal = HGLOBAL(handle.0);
        let size = GlobalSize(hglobal);
        if size < std::mem::size_of::<u32>() {
            return None;
        }
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            return None;
        }
        let val = std::ptr::read_unaligned(ptr as *const u32);
        let _ = GlobalUnlock(hglobal);
        Some(val)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("clipboard is locked by another application and timed out after retries")]
    Locked,
    #[error("clipboard format is unavailable")]
    FormatUnavailable,
    #[error("memory allocation failed: {0}")]
    Memory(windows::core::Error),
    #[error("windows api error: {0}")]
    Api(#[from] windows::core::Error),
}

/// RAII guard ensuring `CloseClipboard` is always invoked.
struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Result<Self, ClipboardError> {
        const RETRIES: usize = 5;
        const DELAY_MS: u64 = 20;

        for i in 0..RETRIES {
            // SAFETY: OpenClipboard with null HWND associates with the current task
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Ok(ClipboardGuard);
            }
            if i + 1 < RETRIES {
                std::thread::sleep(Duration::from_millis(DELAY_MS));
            }
        }
        Err(ClipboardError::Locked)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Get the current clipboard sequence number.
/// Changes whenever the contents of the clipboard change.
pub fn sequence_number() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Attempt to read plain Unicode text from the clipboard.
///
/// Returns `Ok(None)` if the clipboard does not currently contain `CF_UNICODETEXT`.
/// Large payloads exceeding `max_bytes` return `Ok(None)` without allocating huge buffers.
pub fn read_text(max_bytes: usize) -> Result<Option<String>, ClipboardError> {
    let formats = opt_out_formats();

    // Respect Windows standard privacy formats where presence alone indicates skipping
    if (formats.viewer_ignore != 0
        && unsafe { IsClipboardFormatAvailable(formats.viewer_ignore) }.is_ok())
        || (formats.exclude_monitor != 0
            && unsafe { IsClipboardFormatAvailable(formats.exclude_monitor) }.is_ok())
    {
        tracing::debug!("skipping clipboard content marked with Clipboard Viewer Ignore format");
        return Ok(None);
    }

    // Quick format check before locking
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) }.is_err() {
        return Ok(None);
    }

    let _guard = ClipboardGuard::open()?;

    // Respect Windows formats where DWORD 0 means skip
    if formats.can_include_history != 0
        && unsafe { IsClipboardFormatAvailable(formats.can_include_history) }.is_ok()
        && let Some(0) = unsafe { read_clipboard_dword(formats.can_include_history) }
    {
        tracing::debug!("skipping clipboard content marked with CanIncludeInClipboardHistory = 0");
        return Ok(None);
    }

    if formats.can_upload_cloud != 0
        && unsafe { IsClipboardFormatAvailable(formats.can_upload_cloud) }.is_ok()
        && let Some(0) = unsafe { read_clipboard_dword(formats.can_upload_cloud) }
    {
        tracing::debug!("skipping clipboard content marked with CanUploadToCloudClipboard = 0");
        return Ok(None);
    }

    let handle: HANDLE = unsafe { GetClipboardData(CF_UNICODETEXT) }?;

    if handle.is_invalid() {
        return Ok(None);
    }

    let hglobal = HGLOBAL(handle.0);
    let size = unsafe { GlobalSize(hglobal) };
    if size == 0 {
        return Ok(None);
    }

    // Size limit check: size in bytes of UTF-16 data
    if size > max_bytes * 2 {
        tracing::debug!(size, max_bytes, "clipboard data exceeds max size limit");
        return Ok(None);
    }

    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        return Ok(None);
    }

    let u16_ptr = ptr as *const u16;
    let max_u16_len = size / 2;

    // Find null terminator or bound by size
    let mut len = 0;
    while len < max_u16_len && unsafe { *u16_ptr.add(len) } != 0 {
        len += 1;
    }

    let slice = unsafe { std::slice::from_raw_parts(u16_ptr, len) };
    let text = String::from_utf16_lossy(slice);

    unsafe {
        let _ = GlobalUnlock(hglobal);
    }

    Ok(Some(text))
}

/// Write text back to the Windows clipboard.
pub fn write_text(text: &str) -> Result<(), ClipboardError> {
    let _guard = ClipboardGuard::open()?;

    unsafe {
        EmptyClipboard()?;
    }

    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes_len = utf16.len() * std::mem::size_of::<u16>();

    let hmem = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes_len) }.map_err(ClipboardError::Memory)?;

    let ptr = unsafe { GlobalLock(hmem) };
    if ptr.is_null() {
        return Err(ClipboardError::Locked);
    }

    unsafe {
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr as *mut u16, utf16.len());
        let _ = GlobalUnlock(hmem);
    }

    // When SetClipboardData succeeds, system takes ownership of hmem
    unsafe {
        SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0)))?;
    }

    Ok(())
}
