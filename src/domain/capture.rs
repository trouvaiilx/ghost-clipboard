//! Turning raw clipboard text into an entry that may be stored.

use crate::domain::entry::{ContentHash, ContentKind, NewEntry};
use crate::domain::settings::CaptureSettings;

/// Why a piece of clipboard text was not recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// Nothing, or only whitespace, was copied.
    Empty,
    /// The text exceeds the configured size limit.
    TooLarge { bytes: usize, limit: usize },
}

/// Validate `raw` against `settings` and build the entry to store.
///
/// The text is stored **exactly as copied**: line endings (including `\r\n`),
/// indentation and trailing spaces are meaningful in code and logs, and the
/// copy-back path must return what the user originally copied. The only text
/// that is rejected outright is whitespace-only content, which carries nothing
/// worth retrieving.
pub fn prepare(
    raw: String,
    settings: &CaptureSettings,
    source_app: Option<String>,
    now_ms: i64,
) -> Result<NewEntry, Rejection> {
    if raw.trim().is_empty() {
        return Err(Rejection::Empty);
    }
    if raw.len() > settings.max_entry_bytes {
        return Err(Rejection::TooLarge {
            bytes: raw.len(),
            limit: settings.max_entry_bytes,
        });
    }
    Ok(NewEntry {
        kind: ContentKind::detect(&raw),
        hash: ContentHash::of(&raw),
        text: raw,
        copied_at: now_ms,
        source_app,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(limit: usize) -> CaptureSettings {
        CaptureSettings {
            max_entry_bytes: limit,
            ..Default::default()
        }
    }

    fn prep(text: &str) -> Result<NewEntry, Rejection> {
        prepare(text.to_owned(), &settings(1024), None, 42)
    }

    #[test]
    fn empty_and_whitespace_only_text_is_rejected() {
        assert_eq!(prep(""), Err(Rejection::Empty));
        assert_eq!(prep("   \t\r\n  "), Err(Rejection::Empty));
        assert_eq!(prep("\u{00A0}\u{2003}"), Err(Rejection::Empty));
    }

    #[test]
    fn text_is_preserved_exactly() {
        for text in [
            "  leading and trailing  ",
            "line one\r\nline two\r\n",
            "fn main() {\n\tprintln!(\"hi\");\n}\n",
            "tabs\tand  double  spaces",
            "emoji 🎉, accents é, CJK 日本語, RTL שלום",
        ] {
            let entry = prep(text).unwrap();
            assert_eq!(entry.text, text, "text must be stored verbatim");
            assert_eq!(entry.hash, ContentHash::of(text));
        }
    }

    #[test]
    fn oversized_text_is_rejected_not_truncated() {
        let limit = 1024;
        let at_limit = "a".repeat(limit);
        assert!(prepare(at_limit, &settings(limit), None, 0).is_ok());

        let over = "a".repeat(limit + 1);
        assert_eq!(
            prepare(over, &settings(limit), None, 0),
            Err(Rejection::TooLarge {
                bytes: limit + 1,
                limit
            })
        );
    }

    #[test]
    fn the_limit_is_measured_in_utf8_bytes() {
        // 300 three-byte characters are 900 bytes (fits), 400 are 1200 (does not).
        assert!(prepare("日".repeat(300), &settings(1024), None, 0).is_ok());
        assert!(matches!(
            prepare("日".repeat(400), &settings(1024), None, 0),
            Err(Rejection::TooLarge { bytes: 1200, .. })
        ));
    }

    #[test]
    fn metadata_is_attached() {
        let entry = prepare(
            "https://example.com".to_owned(),
            &settings(1024),
            Some("chrome.exe".to_owned()),
            1234,
        )
        .unwrap();
        assert_eq!(entry.kind, ContentKind::Url);
        assert_eq!(entry.copied_at, 1234);
        assert_eq!(entry.source_app.as_deref(), Some("chrome.exe"));
    }
}
