//! Clipboard capture service.
//!
//! Orchestrates clipboard change notifications, sequence tracking,
//! application exclusion filters, size validation, and persistence.

use std::time::SystemTime;

use crate::domain::capture::{Rejection, prepare};
use crate::domain::settings::CaptureSettings;
use crate::platform::{
    ClipboardError, ClipboardSource, ForegroundProbe, SystemClipboard, SystemForeground,
};
use crate::storage::repository::RecordOutcome;
use crate::storage::{Result as StorageResult, StorageHandle};

/// Sensible default exclusion list for known password managers and sensitive apps.
pub const DEFAULT_EXCLUDED_APPS: &[&str] = &[
    "1password.exe",
    "1password7.exe",
    "keepass.exe",
    "keepassxc.exe",
    "bitwarden.exe",
    "enpass.exe",
    "dashlane.exe",
    "nordpass.exe",
    "roboform.exe",
];

pub struct CaptureService<C = SystemClipboard, F = SystemForeground> {
    storage: StorageHandle,
    settings: CaptureSettings,
    paused: bool,
    last_sequence: u32,
    excluded_apps: Vec<String>,
    clipboard: C,
    foreground: F,
}

impl CaptureService<SystemClipboard, SystemForeground> {
    pub fn new(storage: StorageHandle, settings: CaptureSettings) -> Self {
        Self::with_probes(storage, settings, SystemClipboard, SystemForeground)
    }
}

impl<C: ClipboardSource + Clone + 'static, F: ForegroundProbe> CaptureService<C, F> {
    pub fn with_probes(
        storage: StorageHandle,
        settings: CaptureSettings,
        clipboard: C,
        foreground: F,
    ) -> Self {
        let excluded_apps = DEFAULT_EXCLUDED_APPS
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();

        Self {
            storage,
            settings,
            paused: false,
            last_sequence: 0,
            excluded_apps,
            clipboard,
            foreground,
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        tracing::info!(paused, "clipboard capture paused state changed");
    }

    pub fn toggle_paused(&mut self) -> bool {
        self.paused = !self.paused;
        tracing::info!(paused = self.paused, "clipboard capture paused toggled");
        self.paused
    }

    pub fn settings(&self) -> &CaptureSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: CaptureSettings) {
        self.settings = settings.sanitized();
    }

    pub fn excluded_apps(&self) -> &[String] {
        &self.excluded_apps
    }

    pub fn set_excluded_apps(&mut self, apps: Vec<String>) {
        self.excluded_apps = apps.into_iter().map(|s| s.to_ascii_lowercase()).collect();
    }

    pub fn last_sequence(&self) -> u32 {
        self.last_sequence
    }

    /// Explicitly register a sequence number to ignore (e.g. from copy-back operations).
    pub fn ignore_sequence(&mut self, seq: u32) {
        if seq != 0 {
            self.last_sequence = seq;
        }
    }

    /// Process a clipboard change notification.
    ///
    /// Returns `Ok(Some(outcome))` if an entry was recorded,
    /// `Ok(None)` if skipped (paused, sequence unchanged, excluded app, unsupported/empty text),
    /// or `Err(e)` on database/storage failure.
    pub async fn on_clipboard_changed(&mut self) -> StorageResult<Option<RecordOutcome>> {
        if self.paused {
            tracing::debug!("ignoring clipboard change: capture is paused");
            return Ok(None);
        }

        let seq = self.clipboard.sequence_number();
        if seq != 0 && seq == self.last_sequence {
            tracing::debug!(
                seq,
                "ignoring redundant clipboard notification with unchanged sequence"
            );
            return Ok(None);
        }

        let source_app = self.foreground.source_process_name();
        if let Some(ref app) = source_app {
            let app_lower = app.to_ascii_lowercase();
            if self
                .excluded_apps
                .iter()
                .any(|excluded| excluded == &app_lower)
            {
                tracing::debug!(source_app = %app, "ignoring clipboard change from excluded application");
                if seq != 0 {
                    self.last_sequence = seq;
                }
                return Ok(None);
            }
        }

        // Bounded retry for transient clipboard locks (e.g. source app still finalizing write)
        const CAPTURE_RETRIES: usize = 4;
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(25);

        let mut raw_text_opt = None;
        for attempt in 0..CAPTURE_RETRIES {
            let max_bytes = self.settings.max_entry_bytes;
            let clipboard = self.clipboard.clone();
            let read_res =
                tokio::task::spawn_blocking(move || clipboard.read_text(max_bytes)).await;

            match read_res {
                Ok(Ok(text_opt)) => {
                    raw_text_opt = Some(text_opt);
                    break;
                }
                Ok(Err(ClipboardError::Locked)) => {
                    if attempt + 1 < CAPTURE_RETRIES {
                        tokio::time::sleep(RETRY_DELAY).await;
                    } else {
                        tracing::debug!(
                            seq,
                            "clipboard locked after retries; deferring sequence advance for subsequent attempts"
                        );
                        return Ok(None);
                    }
                }
                Ok(Err(err)) => {
                    tracing::debug!(%err, seq, "clipboard read failed; not advancing sequence");
                    return Ok(None);
                }
                Err(join_err) => {
                    tracing::debug!(
                        %join_err,
                        seq,
                        "clipboard read spawn_blocking failed; not advancing sequence"
                    );
                    return Ok(None);
                }
            }
        }

        let raw_text = match raw_text_opt {
            Some(Some(text)) => text,
            Some(None) => {
                tracing::debug!(
                    "clipboard contains no text, opt-out format present, or exceeds limit"
                );
                if seq != 0 {
                    self.last_sequence = seq;
                }
                return Ok(None);
            }
            None => return Ok(None),
        };

        let now_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let new_entry = match prepare(raw_text, &self.settings, source_app, now_ms) {
            Ok(entry) => entry,
            Err(Rejection::Empty) => {
                tracing::debug!("ignoring empty or whitespace-only clipboard text");
                if seq != 0 {
                    self.last_sequence = seq;
                }
                return Ok(None);
            }
            Err(Rejection::TooLarge { bytes, limit }) => {
                tracing::warn!(bytes, limit, "clipboard entry exceeded size limit; skipped");
                if seq != 0 {
                    self.last_sequence = seq;
                }
                return Ok(None);
            }
        };

        let outcome = self.storage.record(new_entry, self.settings.dedup).await?;

        if seq != 0 {
            self.last_sequence = seq;
        }

        // Apply retention policy after recording
        let _ = self
            .storage
            .apply_retention(self.settings.retention, now_ms)
            .await;

        Ok(Some(outcome))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    #[derive(Clone, Default)]
    struct FakeClipboard {
        seq: Arc<AtomicU32>,
        text: Arc<std::sync::Mutex<Option<String>>>,
        locked: Arc<AtomicBool>,
    }

    impl FakeClipboard {
        fn new(seq: u32, text: Option<&str>) -> Self {
            Self {
                seq: Arc::new(AtomicU32::new(seq)),
                text: Arc::new(std::sync::Mutex::new(text.map(|s| s.to_string()))),
                locked: Arc::new(AtomicBool::new(false)),
            }
        }

        fn set(&self, seq: u32, text: Option<&str>) {
            self.seq.store(seq, Ordering::SeqCst);
            *self.text.lock().unwrap() = text.map(|s| s.to_string());
        }
    }

    impl ClipboardSource for FakeClipboard {
        fn sequence_number(&self) -> u32 {
            self.seq.load(Ordering::SeqCst)
        }

        fn read_text(&self, max_bytes: usize) -> Result<Option<String>, ClipboardError> {
            if self.locked.load(Ordering::SeqCst) {
                return Err(ClipboardError::Locked);
            }
            let guard = self.text.lock().unwrap();
            match &*guard {
                Some(text) => {
                    if text.len() > max_bytes {
                        Ok(None)
                    } else {
                        Ok(Some(text.clone()))
                    }
                }
                None => Ok(None),
            }
        }
    }

    #[derive(Clone, Default)]
    struct FakeForeground {
        process_name: Arc<std::sync::Mutex<Option<String>>>,
    }

    impl FakeForeground {
        fn new(name: Option<&str>) -> Self {
            Self {
                process_name: Arc::new(std::sync::Mutex::new(name.map(|s| s.to_string()))),
            }
        }

        fn set(&self, name: Option<&str>) {
            *self.process_name.lock().unwrap() = name.map(|s| s.to_string());
        }
    }

    impl ForegroundProbe for FakeForeground {
        fn source_process_name(&self) -> Option<String> {
            self.process_name.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn test_capture_service_defaults_and_pause() {
        let storage = Storage::spawn_in_memory().unwrap();
        let clip = FakeClipboard::new(0, None);
        let fg = FakeForeground::new(None);
        let mut service =
            CaptureService::with_probes(storage.handle(), CaptureSettings::default(), clip, fg);

        assert!(!service.is_paused());
        assert!(service.toggle_paused());
        assert!(service.is_paused());
        assert!(!service.toggle_paused());
        assert!(!service.is_paused());

        // Check default excluded apps contains major password managers
        assert!(service.excluded_apps().contains(&"keepass.exe".to_string()));
        assert!(
            service
                .excluded_apps()
                .contains(&"1password.exe".to_string())
        );
        assert!(
            service
                .excluded_apps()
                .contains(&"bitwarden.exe".to_string())
        );
        assert!(
            service
                .excluded_apps()
                .contains(&"nordpass.exe".to_string())
        );
        assert!(
            service
                .excluded_apps()
                .contains(&"roboform.exe".to_string())
        );
    }

    #[tokio::test]
    async fn test_capture_records_text_and_advances_sequence() {
        let storage = Storage::spawn_in_memory().unwrap();
        let clip = FakeClipboard::new(10, Some("Captured content from fake"));
        let fg = FakeForeground::new(Some("code.exe"));
        let mut service = CaptureService::with_probes(
            storage.handle(),
            CaptureSettings::default(),
            clip.clone(),
            fg,
        );

        let outcome = service.on_clipboard_changed().await.unwrap();
        assert!(outcome.is_some(), "valid content should be recorded");
        assert_eq!(service.last_sequence(), 10);

        // Verify content in storage
        let report = storage.handle().report().await.unwrap();
        assert_eq!(report.total_entries, 1);

        // Calling again with the same sequence must be skipped without reading
        let redundant = service.on_clipboard_changed().await.unwrap();
        assert_eq!(redundant, None, "unchanged sequence must be skipped");
    }

    #[tokio::test]
    async fn test_capture_excludes_sensitive_applications() {
        let storage = Storage::spawn_in_memory().unwrap();
        let clip = FakeClipboard::new(20, Some("super_secret_master_key"));
        let fg = FakeForeground::new(Some("1Password.exe")); // Mixed case
        let mut service = CaptureService::with_probes(
            storage.handle(),
            CaptureSettings::default(),
            clip.clone(),
            fg.clone(),
        );

        let outcome = service.on_clipboard_changed().await.unwrap();
        assert_eq!(outcome, None, "excluded application must be skipped");
        assert_eq!(
            service.last_sequence(),
            20,
            "sequence must advance on exclusion to prevent re-checking"
        );

        // Verify nothing was recorded
        let report = storage.handle().report().await.unwrap();
        assert_eq!(report.total_entries, 0);

        // Test custom excluded application
        service.set_excluded_apps(vec!["customvault.exe".to_string()]);
        clip.set(21, Some("another_secret"));
        fg.set(Some("CustomVault.EXE"));

        let outcome2 = service.on_clipboard_changed().await.unwrap();
        assert_eq!(outcome2, None);
        assert_eq!(service.last_sequence(), 21);
    }

    #[tokio::test]
    async fn test_capture_paused_skips_and_preserves_sequence() {
        let storage = Storage::spawn_in_memory().unwrap();
        let clip = FakeClipboard::new(30, Some("text while paused"));
        let fg = FakeForeground::new(Some("notepad.exe"));
        let mut service = CaptureService::with_probes(
            storage.handle(),
            CaptureSettings::default(),
            clip.clone(),
            fg,
        );

        service.set_paused(true);
        let outcome = service.on_clipboard_changed().await.unwrap();
        assert_eq!(outcome, None);
        assert_eq!(
            service.last_sequence(),
            0,
            "paused capture must not advance sequence"
        );

        // Unpause: now it captures
        service.set_paused(false);
        let outcome2 = service.on_clipboard_changed().await.unwrap();
        assert!(outcome2.is_some());
        assert_eq!(service.last_sequence(), 30);
    }

    #[tokio::test]
    async fn test_capture_locked_does_not_advance_sequence() {
        let storage = Storage::spawn_in_memory().unwrap();
        let clip = FakeClipboard::new(40, Some("locked text"));
        clip.locked.store(true, Ordering::SeqCst);
        let fg = FakeForeground::new(Some("notepad.exe"));
        let mut service = CaptureService::with_probes(
            storage.handle(),
            CaptureSettings::default(),
            clip.clone(),
            fg,
        );

        let outcome = service.on_clipboard_changed().await.unwrap();
        assert_eq!(outcome, None);
        assert_eq!(
            service.last_sequence(),
            0,
            "locked clipboard failure must not advance sequence permanently"
        );

        // Unlock and retry with same sequence: now it captures
        clip.locked.store(false, Ordering::SeqCst);
        let outcome2 = service.on_clipboard_changed().await.unwrap();
        assert!(outcome2.is_some());
        assert_eq!(service.last_sequence(), 40);
    }

    #[tokio::test]
    async fn test_capture_empty_and_oversized_text_handling() {
        let storage = Storage::spawn_in_memory().unwrap();
        let clip = FakeClipboard::new(50, Some("   \t\n  "));
        let fg = FakeForeground::new(Some("notepad.exe"));
        let mut service = CaptureService::with_probes(
            storage.handle(),
            CaptureSettings::default(),
            clip.clone(),
            fg,
        );

        // Whitespace only is rejected
        let outcome = service.on_clipboard_changed().await.unwrap();
        assert_eq!(outcome, None);
        assert_eq!(
            service.last_sequence(),
            50,
            "definitive rejection advances sequence"
        );

        // Oversized text beyond max_entry_bytes (default 1 MB)
        let oversized = "x".repeat(1024 * 1024 + 100);
        clip.set(51, Some(&oversized));
        let outcome2 = service.on_clipboard_changed().await.unwrap();
        assert_eq!(outcome2, None);
        assert_eq!(service.last_sequence(), 51);
    }

    #[tokio::test]
    async fn test_capture_ignore_sequence_from_copy_back() {
        let storage = Storage::spawn_in_memory().unwrap();
        let clip = FakeClipboard::new(60, Some("copy-back content"));
        let fg = FakeForeground::new(Some("ghostclipboard.exe"));
        let mut service = CaptureService::with_probes(
            storage.handle(),
            CaptureSettings::default(),
            clip.clone(),
            fg,
        );

        // Register ignore sequence before notification
        service.ignore_sequence(60);
        let outcome = service.on_clipboard_changed().await.unwrap();
        assert_eq!(outcome, None, "ignored sequence must be skipped");
        assert_eq!(service.last_sequence(), 60);
    }
}
