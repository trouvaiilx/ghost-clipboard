//! Time and formatting utilities for UI presentation.

use std::time::SystemTime;

/// Format a millisecond Unix timestamp as a human-readable relative time.
pub fn format_relative_time(copied_at_ms: i64, now_ms: i64) -> String {
    if copied_at_ms > now_ms {
        return "just now".to_string();
    }

    let diff_ms = (now_ms - copied_at_ms) as u64;
    let seconds = diff_ms / 1000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let weeks = days / 7;

    if seconds < 45 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if hours < 24 {
        format!("{hours}h ago")
    } else if days == 1 {
        "yesterday".to_string()
    } else if days < 7 {
        format!("{days}d ago")
    } else if weeks < 52 {
        format!("{weeks}w ago")
    } else {
        let years = days / 365;
        format!("{years}y ago")
    }
}

/// Convenience function that uses current system time as `now`.
pub fn format_entry_time(copied_at_ms: i64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(copied_at_ms);
    format_relative_time(copied_at_ms, now_ms)
}

/// Format character count compactly (e.g. `12 chars`, `1.4k chars`).
pub fn format_char_count(chars: usize) -> String {
    if chars < 1000 {
        format!("{chars} chars")
    } else if chars < 1_000_000 {
        format!("{:.1}k chars", chars as f64 / 1000.0)
    } else {
        format!("{:.1}M chars", chars as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_thresholds() {
        let now = 1_000_000_000_000; // ms reference point

        assert_eq!(format_relative_time(now - 10_000, now), "just now");
        assert_eq!(format_relative_time(now - 40_000, now), "just now");
        assert_eq!(format_relative_time(now - 60_000, now), "1m ago");
        assert_eq!(format_relative_time(now - 5 * 60_000, now), "5m ago");
        assert_eq!(format_relative_time(now - 59 * 60_000, now), "59m ago");
        assert_eq!(format_relative_time(now - 60 * 60_000, now), "1h ago");
        assert_eq!(format_relative_time(now - 23 * 3_600_000, now), "23h ago");
        assert_eq!(format_relative_time(now - 25 * 3_600_000, now), "yesterday");
        assert_eq!(format_relative_time(now - 3 * 86_400_000, now), "3d ago");
        assert_eq!(format_relative_time(now - 14 * 86_400_000, now), "2w ago");
        assert_eq!(format_relative_time(now + 10_000, now), "just now"); // clock skew
    }

    #[test]
    fn char_count_formatting() {
        assert_eq!(format_char_count(42), "42 chars");
        assert_eq!(format_char_count(1500), "1.5k chars");
        assert_eq!(format_char_count(2_500_000), "2.5M chars");
    }
}
