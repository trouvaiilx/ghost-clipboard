//! Clipboard history entries.

use sha2::{Digest, Sha256};

/// Number of characters kept in an entry's list preview.
pub const PREVIEW_CHARS: usize = 400;

/// Database identifier of a stored entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId(pub i64);

/// What kind of content an entry holds.
///
/// Only text is captured today. The kind is stored with every entry so images
/// or file references can be added later without rewriting existing history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Text,
    Url,
}

impl ContentKind {
    /// Stable identifier persisted in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            ContentKind::Text => "text",
            ContentKind::Url => "url",
        }
    }

    /// Inverse of [`as_str`](Self::as_str). Unknown values (for example written
    /// by a newer version) degrade to plain text rather than failing to load.
    pub fn from_db(value: &str) -> Self {
        match value {
            "url" => ContentKind::Url,
            _ => ContentKind::Text,
        }
    }

    /// Classify text: a single-line `http(s)://` string without whitespace is a URL.
    pub fn detect(text: &str) -> Self {
        let trimmed = text.trim();
        let lower_prefix_ok = trimmed
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
            || trimmed
                .get(..7)
                .is_some_and(|p| p.eq_ignore_ascii_case("http://"));
        if lower_prefix_ok && !trimmed.contains(char::is_whitespace) {
            ContentKind::Url
        } else {
            ContentKind::Text
        }
    }
}

/// SHA-256 of the exact stored text, used for deduplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    pub fn of(text: &str) -> Self {
        let digest = Sha256::digest(text.as_bytes());
        ContentHash(digest.into())
    }

    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        <[u8; 32]>::try_from(bytes).ok().map(ContentHash)
    }
}

/// An entry ready to be stored. Built by [`crate::domain::capture::prepare`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
    pub text: String,
    pub kind: ContentKind,
    pub hash: ContentHash,
    /// Milliseconds since the Unix epoch.
    pub copied_at: i64,
    /// Executable name of the application that put the text on the clipboard.
    pub source_app: Option<String>,
}

/// A list row. Carries only a bounded preview so listing a large history never
/// loads full clipboard payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySummary {
    pub id: EntryId,
    pub preview: String,
    /// `true` when `preview` is shorter than the stored text.
    pub truncated: bool,
    pub char_count: usize,
    pub kind: ContentKind,
    pub copied_at: i64,
    pub pinned: bool,
    pub source_app: Option<String>,
}

/// A complete entry including its full text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub summary: EntrySummary,
    pub text: String,
}

/// Build the bounded list preview for `text`, returning whether it was cut.
pub fn preview_of(text: &str) -> (String, bool) {
    match text.char_indices().nth(PREVIEW_CHARS) {
        Some((byte_index, _)) => (text[..byte_index].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_urls_conservatively() {
        assert_eq!(
            ContentKind::detect("https://example.com/a?b=c"),
            ContentKind::Url
        );
        assert_eq!(
            ContentKind::detect("  HTTP://Example.com  "),
            ContentKind::Url
        );
        assert_eq!(
            ContentKind::detect("see https://example.com"),
            ContentKind::Text
        );
        assert_eq!(
            ContentKind::detect("https://a.com\nhttps://b.com"),
            ContentKind::Text
        );
        assert_eq!(ContentKind::detect("ftp://example.com"), ContentKind::Text);
        assert_eq!(ContentKind::detect("https://"), ContentKind::Url);
        // Multibyte text must not panic on the prefix slice.
        assert_eq!(
            ContentKind::detect("日本語のテキストです"),
            ContentKind::Text
        );
    }

    #[test]
    fn kind_round_trips_and_unknown_degrades_to_text() {
        for kind in [ContentKind::Text, ContentKind::Url] {
            assert_eq!(ContentKind::from_db(kind.as_str()), kind);
        }
        assert_eq!(ContentKind::from_db("image"), ContentKind::Text);
    }

    #[test]
    fn hash_is_exact_and_whitespace_sensitive() {
        assert_eq!(ContentHash::of("abc"), ContentHash::of("abc"));
        assert_ne!(ContentHash::of("abc"), ContentHash::of("abc "));
        assert_ne!(ContentHash::of("a\nb"), ContentHash::of("a\r\nb"));
        assert_eq!(
            ContentHash::from_slice(&ContentHash::of("x").0),
            Some(ContentHash::of("x"))
        );
        assert_eq!(ContentHash::from_slice(&[1, 2, 3]), None);
    }

    #[test]
    fn short_text_is_not_truncated() {
        let (preview, truncated) = preview_of("hello\nworld");
        assert_eq!(preview, "hello\nworld");
        assert!(!truncated);
    }

    #[test]
    fn preview_truncates_on_a_character_boundary() {
        let text = "é".repeat(PREVIEW_CHARS + 10);
        let (preview, truncated) = preview_of(&text);
        assert!(truncated);
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);

        let exact = "x".repeat(PREVIEW_CHARS);
        assert!(!preview_of(&exact).1, "exactly the limit is not truncated");
    }
}
