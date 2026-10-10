//! Clipboard capture service.
//!
//! Orchestrates clipboard change notifications, sequence tracking,
//! application exclusion filters, size validation, and persistence.

use std::time::SystemTime;

use crate::domain::capture::{Rejection, prepare};
use crate::domain::settings::CaptureSettings;
use crate::platform::windows::{clipboard, foreground};
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

pub struct CaptureService {
    storage: StorageHandle,
    settings: CaptureSettings,
    paused: bool,
    last_sequence: u32,
    excluded_apps: Vec<String>,
}

impl CaptureService {
    pub fn new(storage: StorageHandle, settings: CaptureSettings) -> Self {
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

        let seq = clipboard::sequence_number();
        if seq != 0 && seq == self.last_sequence {
            tracing::debug!(
                seq,
                "ignoring redundant clipboard notification with unchanged sequence"
            );
            return Ok(None);
        }

        let source_app = foreground::source_process_name();
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
            match clipboard::read_text(self.settings.max_entry_bytes) {
                Ok(text_opt) => {
                    raw_text_opt = Some(text_opt);
                    break;
                }
                Err(clipboard::ClipboardError::Locked) => {
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
                Err(err) => {
                    tracing::debug!(%err, seq, "clipboard read failed; not advancing sequence");
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

    static CLIPBOARD_TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn test_capture_service_pause_and_exclusions() {
        let storage = Storage::spawn_in_memory().unwrap();
        let mut service = CaptureService::new(storage.handle(), CaptureSettings::default());

        assert!(!service.is_paused());
        service.set_paused(true);
        assert!(service.is_paused());
        assert_eq!(service.on_clipboard_changed().await.unwrap(), None);

        service.set_paused(false);
        assert!(!service.is_paused());

        // Check default excluded apps
        assert!(service.excluded_apps().contains(&"keepass.exe".to_string()));
        assert!(
            service
                .excluded_apps()
                .contains(&"1password.exe".to_string())
        );
    }

    #[tokio::test]
    async fn test_capture_service_sequence_advance_on_record() {
        let _lock = CLIPBOARD_TEST_MUTEX.lock().await;
        let storage = Storage::spawn_in_memory().unwrap();
        let mut service = CaptureService::new(storage.handle(), CaptureSettings::default());

        let initial_seq = service.last_sequence();
        assert_eq!(initial_seq, 0);

        // Write a test value to clipboard
        let canary = format!(
            "capture_test_{}",
            std::time::SystemTime::now().elapsed().unwrap().as_nanos()
        );
        if clipboard::write_text(&canary).is_ok() {
            let res = service.on_clipboard_changed().await.unwrap();
            assert!(res.is_some(), "valid clipboard text should be recorded");
            assert_ne!(
                service.last_sequence(),
                0,
                "sequence number must advance after definitive recording"
            );

            // Redundant call with unchanged sequence should be skipped immediately
            let redundant = service.on_clipboard_changed().await.unwrap();
            assert_eq!(redundant, None, "redundant sequence must be skipped");
        }
    }

    #[tokio::test]
    async fn test_copy_back_sequence_ignored_prevents_recapture() {
        let _lock = CLIPBOARD_TEST_MUTEX.lock().await;
        let storage = Storage::spawn_in_memory().unwrap();
        let mut service = CaptureService::new(storage.handle(), CaptureSettings::default());

        let canary = format!(
            "copy_back_test_{}",
            std::time::SystemTime::now().elapsed().unwrap().as_nanos()
        );
        if let Ok(seq) = clipboard::write_text(&canary) {
            // Emulate copy-back: tell service to ignore this sequence
            service.ignore_sequence(seq);

            // Clipboard change notification fires
            let outcome = service.on_clipboard_changed().await.unwrap();
            assert_eq!(
                outcome, None,
                "copy-back sequence must be ignored and not re-captured"
            );
        }
    }
}
