use ghost_clipboard::platform::windows::clipboard::{
    CF_UNICODETEXT, opt_out_formats, read_text, sequence_number, write_text,
};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

#[test]
fn opt_out_formats_cached_test() {
    let formats1 = opt_out_formats();
    let formats2 = opt_out_formats();
    assert_ne!(formats1.viewer_ignore, 0);
    assert_ne!(formats1.exclude_monitor, 0);
    assert_ne!(formats1.can_include_history, 0);
    assert_ne!(formats1.can_upload_cloud, 0);
    assert_eq!(formats1.can_include_history, formats2.can_include_history);
}

fn set_clipboard_with_dword_format(text: &str, format: u32, dword_val: u32) -> Result<(), String> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return Err("failed to OpenClipboard".into());
        }
        let _ = EmptyClipboard();

        // Write CF_UNICODETEXT
        let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes_len = utf16.len() * std::mem::size_of::<u16>();
        let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes_len).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(hmem);
        if !ptr.is_null() {
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr as *mut u16, utf16.len());
            let _ = GlobalUnlock(hmem);
            let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0)));
        }

        // Write DWORD format
        let hmem_dword =
            GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of::<u32>()).map_err(|e| e.to_string())?;
        let ptr_dword = GlobalLock(hmem_dword);
        if !ptr_dword.is_null() {
            std::ptr::copy_nonoverlapping(&dword_val as *const u32, ptr_dword as *mut u32, 1);
            let _ = GlobalUnlock(hmem_dword);
            let _ = SetClipboardData(format, Some(HANDLE(hmem_dword.0)));
        }

        let _ = CloseClipboard();
    }
    Ok(())
}

#[test]
fn clipboard_opt_out_formats_test() {
    let formats = opt_out_formats();

    // 1. CanIncludeInClipboardHistory with DWORD = 0 should be skipped
    if let Ok(()) =
        set_clipboard_with_dword_format("secret_history_token", formats.can_include_history, 0)
    {
        let read = read_text(1024 * 1024).expect("read failed");
        assert_eq!(
            read, None,
            "CanIncludeInClipboardHistory = 0 must be skipped"
        );
    }

    // 2. CanIncludeInClipboardHistory with DWORD = 1 should be read
    if let Ok(()) =
        set_clipboard_with_dword_format("allowed_history_token", formats.can_include_history, 1)
    {
        let read = read_text(1024 * 1024).expect("read failed");
        assert_eq!(read.as_deref(), Some("allowed_history_token"));
    }

    // 3. CanUploadToCloudClipboard with DWORD = 0 should be skipped
    if let Ok(()) =
        set_clipboard_with_dword_format("secret_cloud_token", formats.can_upload_cloud, 0)
    {
        let read = read_text(1024 * 1024).expect("read failed");
        assert_eq!(read, None, "CanUploadToCloudClipboard = 0 must be skipped");
    }
}

#[test]
fn clipboard_roundtrip_test() {
    let test_val = format!(
        "ghost_clipboard_test_{}",
        std::time::SystemTime::now().elapsed().unwrap().as_nanos()
    );
    let before_seq = sequence_number();

    // Write to clipboard
    if let Err(e) = write_text(&test_val) {
        eprintln!("Skipping clipboard roundtrip test (cannot open clipboard in this session): {e}");
        return;
    }

    let after_seq = sequence_number();
    assert_ne!(
        before_seq, after_seq,
        "sequence number must change after write"
    );

    // Read back
    let read_back = read_text(1024 * 1024).expect("read failed");
    assert_eq!(read_back.as_deref(), Some(test_val.as_str()));
}
