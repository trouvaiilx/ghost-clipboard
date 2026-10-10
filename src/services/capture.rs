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
        self.last_sequence = seq;

        let source_app = foreground::source_process_name();
        if let Some(ref app) = source_app {
            let app_lower = app.to_ascii_lowercase();
            if self
                .excluded_apps
                .iter()
                .any(|excluded| excluded == &app_lower)
            {
                tracing::info!(source_app = %app, "ignoring clipboard change from excluded application");
                return Ok(None);
            }
        }

        let raw_text = match clipboard::read_text(self.settings.max_entry_bytes) {
            Ok(Some(text)) => text,
            Ok(None) => {
                tracing::debug!("clipboard contains no text or exceeds limit");
                return Ok(None);
            }
            Err(err) => {
                tracing::debug!(%err, "could not read clipboard text");
                return Ok(None);
            }
        };

        let now_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let new_entry = match prepare(raw_text, &self.settings, source_app, now_ms) {
            Ok(entry) => entry,
            Err(Rejection::Empty) => {
                tracing::debug!("ignoring empty or whitespace-only clipboard text");
                return Ok(None);
            }
            Err(Rejection::TooLarge { bytes, limit }) => {
                tracing::warn!(bytes, limit, "clipboard entry exceeded size limit; skipped");
                return Ok(None);
            }
        };

        let outcome = self.storage.record(new_entry, self.settings.dedup).await?;

        // Apply retention policy after recording
        let _ = self
            .storage
            .apply_retention(self.settings.retention, now_ms)
            .await;

        Ok(Some(outcome))
    }
}
