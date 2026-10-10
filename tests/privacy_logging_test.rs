//! Privacy regression test verifying clipboard text and previews NEVER appear in logs.

use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

use ghost_clipboard::domain::settings::CaptureSettings;
use ghost_clipboard::platform::{ClipboardError, ClipboardSource, ForegroundProbe};
use ghost_clipboard::services::capture::CaptureService;
use ghost_clipboard::storage::Storage;

#[derive(Clone, Default)]
struct TestLogWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for TestLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for TestLogWriter {
    type Writer = TestLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[derive(Clone)]
struct FakeClipboard {
    seq: Arc<std::sync::atomic::AtomicU32>,
    text: Arc<Mutex<Option<String>>>,
}

impl ClipboardSource for FakeClipboard {
    fn sequence_number(&self) -> u32 {
        self.seq.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn read_text(&self, _max_bytes: usize) -> Result<Option<String>, ClipboardError> {
        Ok(self.text.lock().unwrap().clone())
    }
}

#[derive(Clone)]
struct FakeForeground {
    app: Arc<Mutex<Option<String>>>,
}

impl ForegroundProbe for FakeForeground {
    fn source_process_name(&self) -> Option<String> {
        self.app.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn test_clipboard_content_and_previews_never_appear_in_logs() {
    let log_buffer = Arc::new(Mutex::new(Vec::new()));
    let writer = TestLogWriter(log_buffer.clone());

    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_max_level(tracing::Level::TRACE)
        .finish();

    let _guard = tracing::subscriber::set_default(subscriber);

    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let clip = FakeClipboard {
        seq: Arc::new(std::sync::atomic::AtomicU32::new(1)),
        text: Arc::new(Mutex::new(Some(
            "SUPER_SENSITIVE_SECRET_TOKEN_DO_NOT_LEAK_99887766".to_string(),
        ))),
    };
    let fg = FakeForeground {
        app: Arc::new(Mutex::new(Some("notepad.exe".to_string()))),
    };

    let mut service = CaptureService::with_probes(
        storage.handle(),
        CaptureSettings::default(),
        clip.clone(),
        fg.clone(),
    );

    // 1. Capture valid sensitive text
    let outcome1 = service.on_clipboard_changed().await.unwrap();
    assert!(outcome1.is_some(), "entry should be captured");

    // 2. Sensitive text from excluded app
    clip.seq.store(2, std::sync::atomic::Ordering::SeqCst);
    *clip.text.lock().unwrap() = Some("MASTER_PASSWORD_FROM_VAULT_DO_NOT_LOG_112233".to_string());
    *fg.app.lock().unwrap() = Some("1Password.exe".to_string());
    let outcome2 = service.on_clipboard_changed().await.unwrap();
    assert!(outcome2.is_none(), "excluded app should be skipped");

    // 3. Sensitive URL
    clip.seq.store(3, std::sync::atomic::Ordering::SeqCst);
    *clip.text.lock().unwrap() =
        Some("https://internal.vault.corp/session?token=CONFIDENTIAL_AUTH_KEY_774411".to_string());
    *fg.app.lock().unwrap() = Some("chrome.exe".to_string());
    let outcome3 = service.on_clipboard_changed().await.unwrap();
    assert!(outcome3.is_some(), "url should be captured");

    // Retrieve captured log contents
    let logs = {
        let raw = log_buffer.lock().unwrap();
        String::from_utf8_lossy(&raw).to_string()
    };

    // Verify tracing actually recorded logs at TRACE level
    assert!(!logs.is_empty(), "logs should contain tracing events");

    // Strictly assert NO sensitive clipboard text or parts appear anywhere in logs
    assert!(
        !logs.contains("SUPER_SENSITIVE_SECRET_TOKEN_DO_NOT_LEAK_99887766"),
        "plain clipboard content was leaked in log output: {logs}"
    );
    assert!(
        !logs.contains("99887766"),
        "clipboard content fragment was leaked in log output: {logs}"
    );
    assert!(
        !logs.contains("MASTER_PASSWORD_FROM_VAULT_DO_NOT_LOG_112233"),
        "excluded app clipboard content was leaked in log output: {logs}"
    );
    assert!(
        !logs.contains("112233"),
        "excluded app content fragment was leaked in log output: {logs}"
    );
    assert!(
        !logs.contains("CONFIDENTIAL_AUTH_KEY_774411"),
        "sensitive URL parameter was leaked in log output: {logs}"
    );
    assert!(
        !logs.contains("774411"),
        "URL token fragment was leaked in log output: {logs}"
    );
}
