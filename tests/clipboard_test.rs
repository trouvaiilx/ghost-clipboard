use ghost_clipboard::platform::windows::clipboard::{read_text, sequence_number, write_text};

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
