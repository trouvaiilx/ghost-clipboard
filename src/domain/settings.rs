//! Policies that govern capture and history size.
//!
//! Persistence of these settings (JSON on disk, a settings screen) arrives in a
//! later phase; the types and their defaults live here so capture and storage
//! can already be driven by them.

use std::time::Duration;

/// How repeated copies of identical text are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DedupMode {
    /// Copying text that already exists anywhere in history moves the existing
    /// entry to the top instead of adding another one. Pin state is kept.
    #[default]
    Anywhere,
    /// Only a copy identical to the *newest* entry is merged into it. The same
    /// text copied again after something else is recorded as a new entry.
    LatestOnly,
}

/// Limits on how much history is kept. Pinned entries are never removed by
/// retention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Keep at most this many *unpinned* entries.
    pub max_entries: Option<usize>,
    /// Remove unpinned entries older than this.
    pub max_age: Option<Duration>,
}

impl RetentionPolicy {
    /// Retain everything.
    pub const UNLIMITED: RetentionPolicy = RetentionPolicy {
        max_entries: None,
        max_age: None,
    };

    /// Entries with `copied_at` (ms since epoch) strictly before this value are expired.
    pub fn age_cutoff(&self, now_ms: i64) -> Option<i64> {
        self.max_age.map(|age| {
            let age_ms = i64::try_from(age.as_millis()).unwrap_or(i64::MAX);
            now_ms.saturating_sub(age_ms)
        })
    }
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        RetentionPolicy {
            max_entries: Some(1000),
            max_age: Some(Duration::from_secs(90 * 24 * 60 * 60)),
        }
    }
}

/// Smallest and largest accepted per-entry size limits.
pub const MIN_ENTRY_BYTES: usize = 1024;
pub const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

/// Everything the capture pipeline needs to decide what to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSettings {
    /// Largest accepted text, in UTF-8 bytes. Larger clipboard contents are
    /// skipped, never truncated: a silently cut-off snippet would be worse
    /// than none.
    pub max_entry_bytes: usize,
    pub dedup: DedupMode,
    pub retention: RetentionPolicy,
}

impl CaptureSettings {
    /// Clamp the size limit into the supported range.
    pub fn sanitized(mut self) -> Self {
        self.max_entry_bytes = self.max_entry_bytes.clamp(MIN_ENTRY_BYTES, MAX_ENTRY_BYTES);
        self
    }
}

impl Default for CaptureSettings {
    fn default() -> Self {
        CaptureSettings {
            max_entry_bytes: 1024 * 1024,
            dedup: DedupMode::default(),
            retention: RetentionPolicy::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative() {
        let settings = CaptureSettings::default();
        assert_eq!(settings.max_entry_bytes, 1024 * 1024);
        assert_eq!(settings.dedup, DedupMode::Anywhere);
        assert_eq!(settings.retention.max_entries, Some(1000));
        assert_eq!(
            settings.retention.max_age,
            Some(Duration::from_secs(90 * 86_400))
        );
    }

    #[test]
    fn size_limit_is_clamped_into_the_supported_range() {
        let low = CaptureSettings {
            max_entry_bytes: 1,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(low.max_entry_bytes, MIN_ENTRY_BYTES);

        let high = CaptureSettings {
            max_entry_bytes: usize::MAX,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(high.max_entry_bytes, MAX_ENTRY_BYTES);
    }

    #[test]
    fn age_cutoff_is_relative_to_now_and_saturates() {
        let policy = RetentionPolicy {
            max_entries: None,
            max_age: Some(Duration::from_secs(10)),
        };
        assert_eq!(policy.age_cutoff(100_000), Some(90_000));
        assert_eq!(RetentionPolicy::UNLIMITED.age_cutoff(100_000), None);

        let huge = RetentionPolicy {
            max_entries: None,
            max_age: Some(Duration::MAX),
        };
        assert_eq!(huge.age_cutoff(5), Some(5 - i64::MAX));
    }
}
